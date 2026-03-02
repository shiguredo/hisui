use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

// 依存ライブラリの名前
const LIB_NAME: &str = "opus";
const SHIGUREDO_OPUS_SYMBOL_PREFIX: &str = "shiguredo_opus_";
const RENAME_TARGET_PREFIXES: &[&str] = &["opus_", "celt_", "clt_", "silk_"];
const TARGET_SOURCE_EXTENSIONS: &[&str] = &["c", "h", "inc", "inl", "s", "S"];

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=CPPFLAGS");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let src_dir = out_build_dir.join(LIB_NAME);
    let input_header_path = src_dir.join("include/opus.h");
    let output_lib_dir = src_dir.join("lib/");
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
    let output_symbol_renames_header_path = out_dir.join("opus_symbol_renames.h");
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

    // ソースツリーから衝突しうるシンボルを収集し、
    // 衝突回避用のリネームヘッダーを生成する
    let symbols = collect_target_identifiers_from_source_tree(&src_dir);
    write_symbol_rename_header(&output_symbol_renames_header_path, &symbols);
    let cppflags = compose_cppflags(&output_symbol_renames_header_path);

    // リネームヘッダーを適用した状態でビルドを行う
    let success = Command::new("./configure")
        .arg("--disable-shared")
        .arg("--prefix")
        .arg(src_dir.display().to_string())
        .env("CPPFLAGS", &cppflags)
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[configure] failed to build {LIB_NAME}");
    }

    let success = Command::new("make")
        .env("CPPFLAGS", &cppflags)
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }

    let success = Command::new("make")
        .arg("install")
        .env("CPPFLAGS", &cppflags)
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }

    // バインディングを生成する
    bindgen::Builder::default()
        .clang_arg("-include")
        .clang_arg(output_symbol_renames_header_path.display().to_string())
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

// Opus のソースツリー全体を走査し、対象プレフィックスの識別子を収集する
//
// リネーム対象プレフィックスに一致する識別子を
// ソースファイル群から抽出する
fn collect_target_identifiers_from_source_tree(src_dir: &Path) -> BTreeSet<String> {
    let mut symbols = BTreeSet::new();
    let mut directories = vec![src_dir.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).expect("failed to read source directory") {
            let entry = entry.expect("failed to read source entry");
            let path = entry.path();
            if entry
                .file_type()
                .expect("failed to read source file type")
                .is_dir()
            {
                directories.push(path);
                continue;
            }
            if !is_target_source_file(&path) {
                continue;
            }
            let source_text = std::fs::read_to_string(&path).expect("failed to read source file");
            for symbol in extract_target_identifiers_from_source(&source_text) {
                symbols.insert(symbol);
            }
        }
    }
    if symbols.is_empty() {
        panic!(
            "failed to collect target symbols from {}",
            src_dir.display()
        );
    }
    symbols
}

// 対象ソースファイルかどうかを判定する
//
// 拡張子で判定している
fn is_target_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| TARGET_SOURCE_EXTENSIONS.contains(&ext))
}

// C ソースを簡易的に正規化し、対象プレフィックス識別子を抽出する
//
// コメントと文字列を空白化してから識別子を走査することで、
// 不要な誤検出を減らしている
fn extract_target_identifiers_from_source(source_text: &str) -> Vec<String> {
    let normalized = normalize_c_source(source_text);
    let mut symbols = BTreeSet::new();
    let mut token = String::new();

    for ch in normalized.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            token.push(ch);
            continue;
        }
        if is_target_symbol(&token) {
            symbols.insert(token.clone());
        }
        token.clear();
    }
    if is_target_symbol(&token) {
        symbols.insert(token);
    }

    symbols.into_iter().collect()
}

// C ソースからコメント・文字列・プリプロセッサ行を除去する
//
// 識別子抽出時のノイズを減らすことが目的
fn normalize_c_source(source_text: &str) -> String {
    let mut output = String::new();
    let mut chars = source_text.chars().peekable();
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_string = false;
    let mut in_char = false;
    let mut escape = false;
    let mut line_start = true;
    let mut in_preprocessor = false;
    let mut preprocessor_continues = false;

    while let Some(ch) = chars.next() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                line_start = true;
                output.push('\n');
            } else {
                output.push(' ');
            }
            continue;
        }
        if in_block_comment {
            if ch == '*' && chars.peek().is_some_and(|next| *next == '/') {
                let _ = chars.next();
                output.push(' ');
                output.push(' ');
                in_block_comment = false;
            } else if ch == '\n' {
                line_start = true;
                output.push('\n');
            } else {
                output.push(' ');
            }
            continue;
        }
        if in_preprocessor {
            if ch == '\n' {
                in_preprocessor = preprocessor_continues;
                preprocessor_continues = false;
                line_start = true;
                output.push('\n');
            } else {
                preprocessor_continues = ch == '\\';
                output.push(' ');
            }
            continue;
        }
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            output.push(' ');
            continue;
        }
        if in_char {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '\'' {
                in_char = false;
            }
            output.push(' ');
            continue;
        }

        if line_start {
            if ch.is_whitespace() {
                output.push(ch);
                line_start = ch == '\n';
                continue;
            }
            if ch == '#' {
                in_preprocessor = true;
                preprocessor_continues = false;
                output.push(' ');
                line_start = false;
                continue;
            }
            line_start = false;
        }

        if ch == '/' && chars.peek().is_some_and(|next| *next == '/') {
            let _ = chars.next();
            in_line_comment = true;
            output.push(' ');
            output.push(' ');
            continue;
        }
        if ch == '/' && chars.peek().is_some_and(|next| *next == '*') {
            let _ = chars.next();
            in_block_comment = true;
            output.push(' ');
            output.push(' ');
            continue;
        }
        if ch == '"' {
            in_string = true;
            output.push(' ');
            continue;
        }
        if ch == '\'' {
            in_char = true;
            output.push(' ');
            continue;
        }
        if ch == '\n' {
            line_start = true;
        }
        output.push(ch);
    }

    output
}

// 対象プレフィックスを持つ C 識別子かどうかを判定する
fn is_target_symbol(symbol: &str) -> bool {
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
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    RENAME_TARGET_PREFIXES
        .iter()
        .any(|prefix| symbol.starts_with(prefix))
}

// シンボルリネーム用ヘッダを生成する
//
// 生成内容:
// - include guard
// - すべての対象シンボルを `shiguredo_opus_<original_symbol>` に変換
fn write_symbol_rename_header(path: &Path, symbols: &BTreeSet<String>) {
    let mut header = String::new();
    header.push_str("#ifndef SHIGUREDO_OPUS_SYMBOL_RENAMES_H\n");
    header.push_str("#define SHIGUREDO_OPUS_SYMBOL_RENAMES_H\n\n");
    for symbol in symbols {
        let renamed_symbol = format!("{SHIGUREDO_OPUS_SYMBOL_PREFIX}{symbol}");
        header.push_str(&format!("#define {symbol} {renamed_symbol}\n"));
    }
    header.push_str("\n#endif\n");
    std::fs::write(path, header).expect("failed to write symbol rename header");
}

// 既存の `CPPFLAGS` を保持しつつ、生成ヘッダの強制インクルード指定を追加する
//
// autotools では `-include` のようなプリプロセッサ関連のフラグは
// `CPPFLAGS` で渡すのが正しい
fn compose_cppflags(include_header: &Path) -> String {
    let include_header = format!("-include {}", include_header.display());
    match std::env::var("CPPFLAGS") {
        Ok(existing) if !existing.trim().is_empty() => format!("{existing} {include_header}"),
        _ => include_header,
    }
}
