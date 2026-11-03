// `sqlx::migrate!` embeds the migrations directory at compile time, but cargo does not know
// that directory is an input. Without this, adding a migration and rebuilding produces a
// binary that silently skips it.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
