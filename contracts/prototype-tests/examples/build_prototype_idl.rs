use std::{error::Error, path::Path};
fn main() -> Result<(), Box<dyn Error>> {
    // anchor-lang-idl 0.1.4 emits literal "+{toolchain}" when Cargo inherits
    // RUSTUP_TOOLCHAIN. This single-threaded helper uses the installed default.
    std::env::remove_var("RUSTUP_TOOLCHAIN");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let idl = anchor_lang_idl::build::IdlBuilder::new()
        .program_path(
            root.join("../prototype/programs/truhabit-prototype")
                .canonicalize()?,
        )
        .cargo_args(vec!["--locked".into()])
        .build()?;
    if idl.address != truhabit_prototype::ID.to_string() || idl.instructions.len() != 2 {
        return Err("Prototype IDL mismatch".into());
    }
    let directory = root.join("../prototype/target/idl");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("truhabit_prototype.json"),
        serde_json::to_string_pretty(&idl)? + "\n",
    )?;
    println!("Prototype IDL generated and checked.");
    Ok(())
}
