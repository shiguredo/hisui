use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

// 依存ライブラリの名前
const LIB_NAME: &str = "opus";
const SHIGUREDO_OPUS_SYMBOL_PREFIX: &str = "shiguredo_opus_";
const SYMBOL_RENAME_HEADER_PATH: &str = "generated/opus_symbol_renames.h";
const SYMBOL_RENAME_INCLUDE_GUARD: &str = "SHIGUREDO_OPUS_SYMBOL_RENAMES_H";
const SYMBOL_REGENERATE_ENV: &str = "SHIGUREDO_OPUS_REGENERATE_SYMBOL_HEADER";
const SYMBOL_SOURCE_DESCRIPTION: &str = "nm -g --defined-only --format=just-symbols libopus.a";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed={SYMBOL_RENAME_HEADER_PATH}");
    println!("cargo::rerun-if-env-changed=CPPFLAGS");
    println!("cargo::rerun-if-env-changed={SYMBOL_REGENERATE_ENV}");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let src_dir = out_build_dir.join(LIB_NAME);
    let input_header_path = src_dir.join("include/opus.h");
    let output_lib_dir = src_dir.join("lib/");
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("infallible"));
    let managed_symbol_renames_header_path = manifest_dir.join(SYMBOL_RENAME_HEADER_PATH);
    let _ = std::fs::remove_dir_all(&out_build_dir);
    std::fs::create_dir(&out_build_dir).expect("failed to create build directory");

    // 各種メタデータを書き込む
    let (git_url, version) = get_git_url_and_version();
    std::fs::write(
        output_metadata_path,
        format!(
            concat!(
                "pub const BUILD_METADATA_REPOSITORY: &str={:?};\n",
                "pub const BUILD_METADATA_VERSION: &str={:?};\n",
            ),
            git_url, version
        ),
    )
    .expect("failed to write metadata file");

    if std::env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは git clone ができないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な構造体だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        std::fs::write(
            output_bindings_path,
            "pub struct OpusEncoder; pub struct OpusDecoder;",
        )
        .expect("write file error");
        return;
    }

    // 依存ライブラリのリポジトリを取得する
    git_clone_external_lib(&out_build_dir);

    // opus の README.md では autogen.sh を呼ぶ手順になっているけど、
    // これだと Hisui では使わないモデルのダウンロードが走って重いので、
    // autoreconf を直接呼ぶようにしている
    let success = Command::new("autoreconf")
        .arg("-isf")
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[autoreconf] failed to build {LIB_NAME}");
    }

    if std::env::var_os(SYMBOL_REGENERATE_ENV).is_some() {
        // 再生成モードでは未リネームの libopus.a を一度ビルドし、
        // 実シンボルから管理ヘッダーを書き出しす
        let cppflags = read_cppflags();
        build_and_install(&src_dir, &cppflags);
        let archive_path = src_dir.join("lib/libopus.a");
        let symbols = collect_symbols_from_archive(&archive_path);
        write_symbol_rename_header(&managed_symbol_renames_header_path, &version, &symbols);
    }

    validate_managed_symbol_rename_header(&managed_symbol_renames_header_path, &version);
    let cppflags = compose_cppflags(read_cppflags(), &managed_symbol_renames_header_path);

    // リネームヘッダーを適用した状態でビルドを行う
    build_and_install(&src_dir, &cppflags);

    // バインディングを生成する
    bindgen::Builder::default()
        .clang_arg("-include")
        .clang_arg(managed_symbol_renames_header_path.display().to_string())
        .header(input_header_path.to_str().expect("invalid header path"))
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    println!("cargo::rustc-link-search={}", output_lib_dir.display());
    println!("cargo::rustc-link-lib=static={LIB_NAME}");
}

// 外部ライブラリのリポジトリを git clone する
fn git_clone_external_lib(build_dir: &Path) {
    let (git_url, version) = get_git_url_and_version();
    let success = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--branch")
        .arg(version)
        .arg(git_url)
        .current_dir(build_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("failed to clone {LIB_NAME} repository");
    }
}

// Cargo.toml から依存ライブラリの Git URL とバージョンタグを取得する
fn get_git_url_and_version() -> (String, String) {
    let cargo_toml: toml::Value =
        toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml");
    if let Some((Some(git_url), Some(version))) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get(LIB_NAME))
        .map(|v| {
            (
                v.get("git").and_then(|s| s.as_str()),
                v.get("version").and_then(|s| s.as_str()),
            )
        })
    {
        (git_url.to_string(), version.to_string())
    } else {
        panic!(
            "Cargo.toml does not contain a valid [package.metadata.external-dependencies.{LIB_NAME}] table"
        );
    }
}

// 管理ヘッダーを生成し、Opus バージョンとシンボル取得方法を先頭コメントに埋め込む
fn write_symbol_rename_header(path: &Path, opus_version: &str, symbols: &BTreeSet<String>) {
    if symbols.is_empty() {
        panic!("symbol rename header requires at least one symbol");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("failed to create symbol rename header directory");
    }

    let mut header = String::new();
    header.push_str("/* Auto-generated by shiguredo_opus build.rs */\n");
    header.push_str(&format!("/* Opus-Version: {opus_version} */\n"));
    header.push_str(&format!(
        "/* Symbol-Source: {SYMBOL_SOURCE_DESCRIPTION} */\n\n"
    ));
    header.push_str(&format!("#ifndef {SYMBOL_RENAME_INCLUDE_GUARD}\n"));
    header.push_str(&format!("#define {SYMBOL_RENAME_INCLUDE_GUARD}\n\n"));
    for symbol in symbols {
        let renamed_symbol = format!("{SHIGUREDO_OPUS_SYMBOL_PREFIX}{symbol}");
        header.push_str(&format!("#define {symbol} {renamed_symbol}\n"));
    }
    header.push_str("\n#endif\n");
    std::fs::write(path, header).expect("failed to write symbol rename header");
}

// 管理ヘッダーの Opus-Version コメントを確認し、期待バージョンと一致しなければ失敗する
fn validate_managed_symbol_rename_header(path: &Path, expected_version: &str) {
    let actual_version = read_managed_header_version(path);
    if actual_version.as_deref() == Some(expected_version) {
        return;
    }
    let actual_version = actual_version.unwrap_or_else(|| "(missing or unknown)".to_string());
    panic!(
        concat!(
            "管理ヘッダーが見つからないか、Opus-Version が一致しません。\n",
            "期待する Opus-Version: {expected}\n",
            "実際の Opus-Version: {actual}\n",
            "ヘッダーパス: {path}\n",
            "再生成コマンド:\n",
            "  SHIGUREDO_OPUS_REGENERATE_SYMBOL_HEADER=1 cargo build -p shiguredo_opus"
        ),
        expected = expected_version,
        actual = actual_version,
        path = path.display(),
    );
}

// 管理ヘッダーの先頭コメントから Opus-Version を読み取る
fn read_managed_header_version(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("/* Opus-Version: ") {
            return rest.strip_suffix(" */").map(ToString::to_string);
        }
    }
    None
}

// `nm` 出力から定義済みグローバルシンボルを抽出する
fn collect_symbols_from_archive(archive_path: &Path) -> BTreeSet<String> {
    let output = Command::new("nm")
        .arg("-g")
        .arg("--defined-only")
        .arg("--format=just-symbols")
        .arg(archive_path)
        .output()
        .expect("failed to run nm");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("nm failed for {}: {stderr}", archive_path.display());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut symbols = BTreeSet::new();
    for line in stdout.lines() {
        let symbol = line.trim();
        if is_c_identifier(symbol) && !symbol.starts_with(SHIGUREDO_OPUS_SYMBOL_PREFIX) {
            symbols.insert(symbol.to_string());
        }
    }
    if symbols.is_empty() {
        panic!("no symbols were collected from {}", archive_path.display());
    }
    symbols
}

// C 識別子かどうかを判定する
fn is_c_identifier(symbol: &str) -> bool {
    if symbol.is_empty() {
        return false;
    }
    let mut chars = symbol.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// configure / make / make install を行う
fn build_and_install(src_dir: &Path, cppflags: &str) {
    let mut configure = Command::new("./configure");
    configure
        .arg("--disable-shared")
        .arg("--prefix")
        .arg(src_dir.display().to_string())
        .current_dir(src_dir)
        .env("CPPFLAGS", cppflags);
    let success = configure.status().is_ok_and(|status| status.success());
    if !success {
        panic!("[configure] failed to build {LIB_NAME}");
    }

    let mut make = Command::new("make");
    make.current_dir(src_dir).env("CPPFLAGS", cppflags);
    let success = make.status().is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }

    let mut make_install = Command::new("make");
    make_install
        .arg("install")
        .current_dir(src_dir)
        .env("CPPFLAGS", cppflags);
    let success = make_install.status().is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }
}

// `CPPFLAGS` 環境変数をそのまま取得する
fn read_cppflags() -> String {
    std::env::var("CPPFLAGS").unwrap_or_default()
}

// 既存の `CPPFLAGS` に生成ヘッダの強制インクルード指定を追加する
fn compose_cppflags(cppflags: String, include_header: &Path) -> String {
    format!("{} -include {}", cppflags, include_header.display())
}
