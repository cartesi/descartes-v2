use ethers::contract::Abigen;
use serde_json::Value;

fn write_contract(
    contract_name: &str,
    source: &str,
    destination: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let s = std::fs::read_to_string(source)?;
    let v: Value = serde_json::from_str(&s)?;
    let abi_str = serde_json::to_string(&v["abi"])?;

    let bindings = Abigen::new(&contract_name, abi_str)?.generate()?;

    bindings.write_to_file(destination)?;

    let cargo_rerun = "cargo:rerun-if-changed";
    println!("{}={}", cargo_rerun, source);
    println!("{}={}", cargo_rerun, destination);

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let contracts = vec![
        (
            "DescartesV2Impl",
            "DescartesV2Impl",
            "descartesv2_contract.rs",
        ),
        ("InputImpl", "InputImpl", "input_contract.rs"),
    ];

    for (name, file, rs) in contracts {
        let path = format!("../artifacts/contracts/{}.sol/{}.json", file, name);
        let destination = format!("./src/contracts/{}", rs);
        write_contract(name, &path, &destination)?;
    }

    tonic_build::configure().build_server(false).compile(
        &[
            "../grpc-interfaces/versioning.proto",
            "../grpc-interfaces/cartesi-machine.proto",
            "../grpc-interfaces/rollup-machine-manager.proto",
        ],
        &["../grpc-interfaces"],
    )?;

    println!("cargo:rerun-if-changed=../grpc-interfaces/versioning.proto");
    println!("cargo:rerun-if-changed=../grpc-interfaces/cartesi-machine.proto");
    println!("cargo:rerun-if-changed=../grpc-interfaces/rollup-machine-manager.proto");

    println!("cargo:rerun-if-changed=build.rs");

    Ok(())
}
