use offchain_core::ethers;

use crate::contracts::descartesv2_contract::*;

use super::epoch_delegate::{ContractPhase, EpochFoldDelegate, EpochState};
use super::sealed_epoch_delegate::SealedEpochState;
use super::types::{
    AccumulatingEpoch, DescartesV2State, ImmutableState, PhaseState,
};

use offchain_core::types::Block;
use state_fold::{
    delegate_access::{FoldAccess, SyncAccess},
    error::*,
    types::*,
    DelegateAccess, StateFold,
};

use async_trait::async_trait;
use snafu::ResultExt;
use std::sync::Arc;

use ethers::providers::Middleware;
use ethers::types::{Address, U256};

/// DescartesV2 StateActor Delegate, which implements `sync` and `fold`.
pub struct DescartesV2FoldDelegate<DA: DelegateAccess + Send + Sync + 'static> {
    descartesv2_address: Address,
    epoch_fold: Arc<StateFold<EpochFoldDelegate<DA>, DA>>,
}

impl<DA: DelegateAccess + Send + Sync + 'static> DescartesV2FoldDelegate<DA> {
    pub fn new(
        descartesv2_address: Address,
        epoch_fold: Arc<StateFold<EpochFoldDelegate<DA>, DA>>,
    ) -> Self {
        Self {
            descartesv2_address,
            epoch_fold,
        }
    }
}

#[async_trait]
impl<DA: DelegateAccess + Send + Sync + 'static> StateFoldDelegate
    for DescartesV2FoldDelegate<DA>
{
    type InitialState = U256; // Initial epoch
    type Accumulator = DescartesV2State;
    type State = BlockState<Self::Accumulator>;

    async fn sync<A: SyncAccess + Send + Sync>(
        &self,
        initial_state: &U256,
        block: &Block,
        access: &A,
    ) -> SyncResult<Self::Accumulator, A> {
        let middleware = access
            .build_sync_contract(Address::zero(), block.number, |_, m| m)
            .await;

        let contract = DescartesV2Impl::new(
            self.descartesv2_address,
            Arc::clone(&middleware),
        );

        // Retrieve constants from contract creation event
        let constants = {
            let (create_event, meta) = {
                let e = contract
                    .descartes_v2_created_filter()
                    .query_with_meta()
                    .await
                    .context(SyncContractError {
                        err: "Error querying for descartes created",
                    })?;

                if e.is_empty() {
                    return SyncDelegateError {
                        err: "Descartes create event not found",
                    }
                    .fail();
                }

                assert_eq!(e.len(), 1);
                e[0].clone()
            };

            // retrieve timestamp of creation
            let timestamp = middleware
                .get_block(meta.block_hash)
                .await
                .context(SyncAccessError {})?
                .ok_or(snafu::NoneError)
                .context(SyncDelegateError {
                    err: "Block not found",
                })?
                .timestamp;

            ImmutableState::from(&(create_event, timestamp))
        };

        // get raw state from EpochFoldDelegate
        let raw_contract_state = self
            .epoch_fold
            .get_state_for_block(initial_state, block.hash)
            .await
            .map_err(|e| {
                SyncDelegateError {
                    err: format!("Epoch state fold error: {:?}", e),
                }
                .build()
            })?
            .state;

        Ok(convert_raw_to_logical(
            raw_contract_state,
            constants,
            block,
            initial_state,
        ))
    }

    async fn fold<A: FoldAccess + Send + Sync>(
        &self,
        previous_state: &Self::Accumulator,
        block: &Block,
        _access: &A,
    ) -> FoldResult<Self::Accumulator, A> {
        let constants = previous_state.constants.clone();

        // get raw state from EpochFoldDelegate
        let raw_contract_state = self
            .epoch_fold
            .get_state_for_block(&previous_state.initial_epoch, block.hash)
            .await
            .map_err(|e| {
                FoldDelegateError {
                    err: format!("Epoch state fold error: {:?}", e),
                }
                .build()
            })?
            .state;

        Ok(convert_raw_to_logical(
            raw_contract_state,
            constants,
            block,
            &previous_state.initial_epoch,
        ))
    }

    fn convert(
        &self,
        accumulator: &BlockState<Self::Accumulator>,
    ) -> Self::State {
        accumulator.clone()
    }
}

// Convert raw state to logical state. Raw state is the literal interpretation
// of what is being presented by the blockchain. Logical state is the semantic
// intepretation of that, which will be used for offchain decision making
fn convert_raw_to_logical(
    contract_state: EpochState,
    constants: ImmutableState,
    block: &Block,
    initial_epoch: &U256,
) -> DescartesV2State {
    // If the raw state is InputAccumulation but it has expired, then the raw
    // state's `current_epoch` becomes the sealed epoch, and the logic state's
    // `current_epoch` is empty.
    // This variable contains `Some(epoch_number)` in this case, and `None`
    // otherwise.
    // This is possible because a new input after InputAccumulation has expired
    // would trigger a phase change to AwaitingConsensus.
    let mut current_epoch_no_inputs: Option<U256> = None;

    let phase_state = match contract_state.current_phase {
        ContractPhase::InputAccumulation {} => {
            // Last phase change timestamp is the timestamp of input
            // accumulation start if contract in InputAccumulation.
            // If there were no phase changes, it is the timestamp of
            // contract creation.
            let input_accumulation_start_timestamp =
                if let Some(ts) = contract_state.phase_change_timestamp {
                    ts
                } else {
                    constants.contract_creation_timestamp
                };

            // If input duration has passed, the logical state is epoch sealed
            // awaiting first claim. The raw state can still be InputAccumulation
            // if there were no new inputs after the phase expired.
            if block.timestamp
                > input_accumulation_start_timestamp + constants.input_duration
            {
                current_epoch_no_inputs =
                    Some(contract_state.current_epoch.epoch_number + 1);
                PhaseState::EpochSealedAwaitingFirstClaim {
                    sealed_epoch: contract_state.current_epoch.clone(),
                }
            } else {
                PhaseState::InputAccumulation {}
            }
        }

        ContractPhase::AwaitingConsensus {
            sealed_epoch,
            round_start,
        } => {
            // The raw phase change might have happened because a claim arrived
            // or because a new input arrived. This determines if the logical
            // phase is EpochAwaintFirstClaim or SealedEpochNoClaims
            match sealed_epoch {
                SealedEpochState::SealedEpochNoClaims { sealed_epoch } => {
                    PhaseState::EpochSealedAwaitingFirstClaim { sealed_epoch }
                }

                SealedEpochState::SealedEpochWithClaims { claimed_epoch } => {
                    let first_claim_timestamp =
                        claimed_epoch.claims.first_claim_timestamp();

                    // We can safely unwrap because we can be sure
                    // there was at least one phase change event.
                    // let phase_change_timestamp =
                    //     contract_state.phase_change_timestamp.unwrap();
                    let phase_change_timestamp = round_start;

                    // Last move's timestamp is the most recent timestamp between
                    // the first claim or the phase change. This happens because
                    // the 'challenge period' starts on first claim but resets
                    // after a dispute.
                    let time_of_last_move = std::cmp::max(
                        first_claim_timestamp,
                        phase_change_timestamp,
                    );

                    // Check if Consensus timed out or, using the first claim
                    // timestamp variable, decide if this is the first challenge
                    // period of this epoch or if it is posterior to a dispute
                    if block.timestamp
                        > time_of_last_move + constants.challenge_period
                    {
                        PhaseState::ConsensusTimeout { claimed_epoch }
                    } else if time_of_last_move == first_claim_timestamp {
                        PhaseState::AwaitingConsensusNoConflict {
                            claimed_epoch,
                        }
                    } else {
                        PhaseState::AwaitingConsensusAfterConflict {
                            claimed_epoch,
                            challenge_period_base_ts: phase_change_timestamp,
                        }
                    }
                }
            }
        }

        // This version doesn't have disputes. They're resolved automatically
        // onchain
        ContractPhase::AwaitingDispute { .. } => {
            unreachable!()
        }
    };

    // Figures out if the current accumulating epoch is empty (new) or if it
    // was previously created. The distinction comes from the two possible
    // transitions to AwaitingConsensus, either a new input or a claim
    let current_epoch = if let Some(epoch_number) = current_epoch_no_inputs {
        AccumulatingEpoch::new(epoch_number)
    } else {
        contract_state.current_epoch
    };

    DescartesV2State {
        constants,
        initial_epoch: *initial_epoch,
        current_phase: phase_state,
        finalized_epochs: contract_state.finalized_epochs,
        current_epoch,
    }
}

// Fetches the DescartesV2 constants from the contract creation event
impl From<&(DescartesV2CreatedFilter, U256)> for ImmutableState {
    fn from(src: &(DescartesV2CreatedFilter, U256)) -> Self {
        let (ev, ts) = src;
        Self {
            input_duration: ev.input_duration,
            challenge_period: ev.challenge_period,
            contract_creation_timestamp: ts.clone(),
            input_contract_address: ev.input,
            output_contract_address: ev.output,
            validator_contract_address: ev.validator_manager,
            dispute_contract_address: ev.dispute_manager,
        }
    }
}
