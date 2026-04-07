use svault_core::db;

pub fn run() -> anyhow::Result<()> {
    let root = std::env::current_dir().expect("cannot read cwd");
    db::init(&root)?;
    println!("Initialized empty svault at {}", root.join(".svault").display());
    Ok(())
}
