//! `nxf-fold-purge`: delete one stream's partition of the board-fold table (6j6v.dbmd). The tool is
//! `nxs_fold_ddb::purge::cli`; this file only hands it the process's arguments, environment and
//! streams, and exits with its code.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |name: &str| std::env::var(name).ok();
    let code = nxs_fold_ddb::purge::cli::run(
        &args,
        &env,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr(),
    );
    std::process::exit(i32::from(code));
}
