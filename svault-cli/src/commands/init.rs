use svault_core::db;
use svault_ui::messages;

pub fn run() -> anyhow::Result<()> {
    let root = std::env::current_dir().expect("cannot read cwd");
    db::init(&root)?;
    messages::success(&format!(
        "Initialized empty svault at {}",
        root.join(".svault").display()
    ));
    Ok(())
}
