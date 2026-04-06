use svault_core::context::find_vault_root;

pub fn run() -> anyhow::Result<()> {
    let vault_root = find_vault_root(None, &std::env::current_dir()?)?;
    let _lock = svault_core::lock::acquire_vault_lock(&vault_root)?;
    todo!("clone")
}
