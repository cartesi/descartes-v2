use offchain_core::ethers;

use super::*;
use state_fold::{DelegateAccess, StateFold};

use ethers::core::types::Address;

use std::sync::Arc;

pub struct SetupConfig {
    pub safety_margin: usize,
    pub input_contract_address: Address,
    pub descartes_contract_address: Address,
}

pub type DescartesStateFold<DA> =
    Arc<StateFold<DescartesV2FoldDelegate<DA>, DA>>;

/// Creates DescartesV2 State Fold
pub fn create_descartes_state_fold<
    DA: DelegateAccess + Send + Sync + 'static,
>(
    access: Arc<DA>,
    config: &SetupConfig,
) -> DescartesStateFold<DA> {
    let epoch_fold = create_epoch(Arc::clone(&access), config);

    let delegate = DescartesV2FoldDelegate::new(
        config.descartes_contract_address,
        epoch_fold,
    );
    let state_fold = StateFold::new(delegate, access, config.safety_margin);
    Arc::new(state_fold)
}

type InputStateFold<DA> = Arc<StateFold<InputFoldDelegate, DA>>;
fn create_input<DA: DelegateAccess + Send + Sync + 'static>(
    access: Arc<DA>,
    config: &SetupConfig,
) -> InputStateFold<DA> {
    let delegate = InputFoldDelegate::new(config.input_contract_address);
    let state_fold = StateFold::new(delegate, access, config.safety_margin);
    Arc::new(state_fold)
}

type AccumulatingEpochStateFold<DA> =
    Arc<StateFold<AccumulatingEpochFoldDelegate<DA>, DA>>;
fn create_accumulating_epoch<DA: DelegateAccess + Send + Sync + 'static>(
    input_fold: InputStateFold<DA>,
    access: Arc<DA>,
    config: &SetupConfig,
) -> AccumulatingEpochStateFold<DA> {
    let delegate = AccumulatingEpochFoldDelegate::new(input_fold);
    let state_fold = StateFold::new(delegate, access, config.safety_margin);
    Arc::new(state_fold)
}

type SealedEpochStateFold<DA> = Arc<StateFold<SealedEpochFoldDelegate<DA>, DA>>;
fn create_sealed_epoch<DA: DelegateAccess + Send + Sync + 'static>(
    input_fold: InputStateFold<DA>,
    access: Arc<DA>,
    config: &SetupConfig,
) -> SealedEpochStateFold<DA> {
    let delegate = SealedEpochFoldDelegate::new(
        config.descartes_contract_address,
        input_fold,
    );
    let state_fold = StateFold::new(delegate, access, config.safety_margin);
    Arc::new(state_fold)
}

type FinalizedEpochStateFold<DA> =
    Arc<StateFold<FinalizedEpochFoldDelegate<DA>, DA>>;
fn create_finalized_epoch<DA: DelegateAccess + Send + Sync + 'static>(
    input_fold: InputStateFold<DA>,
    access: Arc<DA>,
    config: &SetupConfig,
) -> FinalizedEpochStateFold<DA> {
    let delegate = FinalizedEpochFoldDelegate::new(
        config.descartes_contract_address,
        input_fold,
    );
    let state_fold = StateFold::new(delegate, access, config.safety_margin);
    Arc::new(state_fold)
}

type EpochStateFold<DA> = Arc<StateFold<EpochFoldDelegate<DA>, DA>>;
fn create_epoch<DA: DelegateAccess + Send + Sync + 'static>(
    access: Arc<DA>,
    config: &SetupConfig,
) -> EpochStateFold<DA> {
    let input_fold = create_input(Arc::clone(&access), config);
    let accumulating_fold = create_accumulating_epoch(
        Arc::clone(&input_fold),
        Arc::clone(&access),
        config,
    );
    let sealed_fold = create_sealed_epoch(
        Arc::clone(&input_fold),
        Arc::clone(&access),
        config,
    );
    let finalized_fold = create_finalized_epoch(
        Arc::clone(&input_fold),
        Arc::clone(&access),
        config,
    );

    let delegate = EpochFoldDelegate::new(
        config.descartes_contract_address,
        accumulating_fold,
        sealed_fold,
        finalized_fold,
    );
    let state_fold = StateFold::new(delegate, access, config.safety_margin);
    Arc::new(state_fold)
}
