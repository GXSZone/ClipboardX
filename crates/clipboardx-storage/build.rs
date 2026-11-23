//! Builds the SQLite3 Multiple Ciphers (sqlite3mc) amalgamation statically with FTS5 enabled.

fn main() {
    let dir = sqlite3mc_src::source_dir();
    let source = dir.join(sqlite3mc_src::SOURCE_FILE);

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:include={}", dir.display());

    let mut build = cc::Build::new();
    build
        .file(&source)
        .include(dir)
        .define("SQLITE_CORE", None)
        .define("SQLITE_DEFAULT_FOREIGN_KEYS", Some("1"))
        .define("SQLITE_ENABLE_API_ARMOR", None)
        .define("SQLITE_ENABLE_FTS5", None)
        .define("SQLITE_ENABLE_JSON1", None)
        .define("SQLITE_ENABLE_RTREE", None)
        .define("SQLITE_THREADSAFE", Some("1"))
        .define("SQLITE_USE_URI", None)
        .define("SQLITE_TEMP_STORE", Some("2"))
        .define("CODEC_TYPE", Some("CODEC_TYPE_CHACHA20"))
        .warnings(false);

    if !cfg!(target_os = "windows") {
        build.define("HAVE_USLEEP", Some("1"));
    }

    build.compile("sqlite3");
}
