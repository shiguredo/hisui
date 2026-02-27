use std::{
    path::{Path, PathBuf},
    process::Command,
};

// 依存ライブラリの名前
const LIB_NAME: &str = "opus";

// シンボルプレフィックス
// shiguredo_webrtc の libwebrtc_c.a にも opus が含まれているため、
// シンボル名にプレフィックスを付けて衝突を防ぐ。
const SYMBOL_PREFIX: &str = "shiguredo_";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("infallible"));
    let out_build_dir = out_dir.join("build/");
    let src_dir = out_build_dir.join(LIB_NAME);
    let input_header_path = src_dir.join("include/opus.h");
    let output_lib_dir = src_dir.join("lib/");
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");
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

    // 依存ライブラリをビルドする

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

    let success = Command::new("./configure")
        .arg("--disable-shared")
        .arg("--prefix")
        .arg(src_dir.display().to_string())
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[configure] failed to build {LIB_NAME}");
    }

    let success = Command::new("make")
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }

    let success = Command::new("make")
        .arg("install")
        .current_dir(&src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }

    // Linux では ld -r で部分リンクし、objcopy でシンボルにプレフィックスを付ける。
    // shiguredo_webrtc の libwebrtc_c.a にも opus が含まれているため、
    // シンボル名を変えて衝突を防ぐ。
    if cfg!(target_os = "linux") {
        prefix_symbols(&output_lib_dir);
    }

    // バインディングを生成する
    bindgen::Builder::default()
        .header(input_header_path.to_str().expect("invalid header path"))
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(&output_bindings_path)
        .expect("failed to write bindings");

    // Linux ではシンボルにプレフィックスを付けているので、
    // バインディングの extern 関数にも #[link_name] 属性を追加する
    if cfg!(target_os = "linux") {
        add_link_name_prefix(&output_bindings_path, SYMBOL_PREFIX);
    }

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

// 定義済みグローバルシンボルにプレフィックスを付ける
//
// 1. ld -r --whole-archive で全オブジェクトを1つに結合し、内部参照を解決する
// 2. nm で定義済みグローバルシンボルの一覧を取得する
// 3. objcopy --redefine-syms でシンボル名にプレフィックスを付ける
//    (--prefix-symbols だと未定義シンボル(libc 等)まで変わるため使えない)
// 4. ar で新しいアーカイブを作成する
fn prefix_symbols(lib_dir: &Path) {
    let lib_path = lib_dir.join("libopus.a");
    let merged_obj = lib_dir.join("opus_merged.o");
    let redefine_syms_path = lib_dir.join("redefine_syms.txt");

    // 全オブジェクトを1つに結合して内部参照を解決する
    let success = Command::new("ld")
        .arg("-r")
        .arg("--whole-archive")
        .arg(&lib_path)
        .arg("-o")
        .arg(&merged_obj)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[ld -r] failed to merge objects in libopus.a");
    }

    // 定義済みグローバルシンボルの一覧を取得し、リネームマップを作成する
    let nm_output = Command::new("nm")
        .arg("--defined-only")
        .arg("-g")
        .arg(&merged_obj)
        .output()
        .expect("[nm] failed to execute");
    assert!(nm_output.status.success(), "[nm] failed to list symbols");

    let nm_stdout = String::from_utf8_lossy(&nm_output.stdout);
    let mut redefine_content = String::new();
    for line in nm_stdout.lines() {
        // nm の出力形式: "address type name"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && !parts[2].is_empty() {
            let name = parts[2];
            redefine_content.push_str(&format!("{name} {SYMBOL_PREFIX}{name}\n"));
        }
    }
    std::fs::write(&redefine_syms_path, &redefine_content)
        .expect("failed to write redefine_syms.txt");

    // 定義済みグローバルシンボルだけをリネームする
    let success = Command::new("objcopy")
        .arg(format!("--redefine-syms={}", redefine_syms_path.display()))
        .arg(&merged_obj)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[objcopy] failed to rename symbols");
    }

    // 新しいアーカイブを作成する
    let _ = std::fs::remove_file(&lib_path);
    let success = Command::new("ar")
        .arg("rcs")
        .arg(&lib_path)
        .arg(&merged_obj)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[ar] failed to create new libopus.a");
    }
}

// バインディングの extern 関数に #[link_name] 属性を追加する
//
// bindgen が生成するバインディングは元のシンボル名（例: opus_encode）を使うが、
// libopus.a 内のシンボルはプレフィックス済み（例: shiguredo_opus_encode）なので、
// #[link_name] 属性でリンカに正しいシンボル名を伝える。
fn add_link_name_prefix(bindings_path: &Path, prefix: &str) {
    let content = std::fs::read_to_string(bindings_path).expect("failed to read bindings file");
    let mut result = String::with_capacity(content.len() * 2);

    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(after_pub_fn) = trimmed.strip_prefix("pub fn ") {
            if let Some(name) = after_pub_fn.split('(').next() {
                let indent = &line[..line.len() - trimmed.len()];
                result.push_str(&format!("{indent}#[link_name = \"{prefix}{name}\"]\n"));
            }
        }
        result.push_str(line);
        result.push('\n');
    }

    std::fs::write(bindings_path, result).expect("failed to write modified bindings file");
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
