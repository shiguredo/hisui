use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use bindgen::callbacks::{ItemInfo, ItemKind, ParseCallbacks};

#[derive(Debug)]
struct SymbolLinkNameCallbacks {
    link_name_map: BTreeMap<String, String>,
}

impl SymbolLinkNameCallbacks {
    pub fn new(link_name_map: BTreeMap<String, String>) -> Self {
        Self { link_name_map }
    }
}

impl ParseCallbacks for SymbolLinkNameCallbacks {
    fn generated_link_name_override(&self, item_info: ItemInfo<'_>) -> Option<String> {
        match item_info.kind {
            ItemKind::Function | ItemKind::Var => self.link_name_map.get(item_info.name).cloned(),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct LlvmTools {
    nm: PathBuf,
    objcopy: PathBuf,
}

#[derive(Debug)]
struct SymbolRenameMaps {
    // objcopy の --redefine-syms で使う、元シンボル名 -> 書き換え後シンボル名 のマップ
    // 例: `opus_encode` -> `shiguredo_opus_encode`
    // 例: `_opus_encode` -> `_shiguredo_opus_encode` (macOS の場合)
    objcopy_rename_map: BTreeMap<String, String>,
    // bindgen で使う、元シンボル名 -> 書き換え後シンボル名 のマップ
    // 例: `opus_encode` -> `shiguredo_opus_encode`
    // 例: `opus_encode` -> `_shiguredo_opus_encode` (macOS の場合)
    //
    // objcopy_rename_map は macOS の場合に装飾付きシンボル名（先頭 `_` あり）キーにしているのに対して、
    // link_name_map は '_' を取り除いて正規化したシンボル名をキーにしている
    link_name_map: BTreeMap<String, String>,
}

// 静的ライブラリのシンボルを書き換えて、bindgen 用の callback を返す
// rename_symbol が None を返したシンボルは書き換え対象から除外する
pub fn rewrite_symbols<F>(
    static_library_path: &Path,
    rename_symbol: F,
    temporary_files_dir: &Path,
    is_macos: bool,
) -> Box<dyn ParseCallbacks>
where
    F: Fn(&str) -> Option<String>,
{
    let objcopy_rename_map_path = temporary_files_dir.join("symbol_rename_map.txt");
    let llvm_tools = discover_llvm_tools();
    let platform_symbols = collect_defined_external_symbols(&llvm_tools.nm, static_library_path);
    let symbol_maps = build_symbol_rename_maps(platform_symbols, &rename_symbol, is_macos);

    if symbol_maps.objcopy_rename_map.is_empty() {
        panic!("no symbols were collected for symbol rewriting");
    }

    write_objcopy_rename_map(&objcopy_rename_map_path, &symbol_maps.objcopy_rename_map);

    rewrite_archive_symbols(
        &llvm_tools.objcopy,
        static_library_path,
        &objcopy_rename_map_path,
    );

    // 一時ファイルなので後始末する。失敗しても本処理には影響させない。
    let _ = std::fs::remove_file(&objcopy_rename_map_path);

    Box::new(SymbolLinkNameCallbacks::new(symbol_maps.link_name_map))
}

// rustup の llvm-tools から llvm-nm / llvm-objcopy のパスを解決する
fn discover_llvm_tools() -> LlvmTools {
    let host = std::env::var("HOST").expect("HOST environment variable is not set");
    let sysroot = get_rustc_sysroot();
    let bin_dir = sysroot.join("lib").join("rustlib").join(host).join("bin");

    let nm = bin_dir.join(exe_name("llvm-nm"));
    let objcopy = bin_dir.join(exe_name("llvm-objcopy"));

    if !nm.is_file() || !objcopy.is_file() {
        panic!(concat!(
            "llvm tools were not found in Rust sysroot. ",
            "Please install them with: rustup component add llvm-tools"
        ));
    }

    LlvmTools { nm, objcopy }
}

// rustc --print sysroot の結果を取得する
fn get_rustc_sysroot() -> PathBuf {
    let output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("failed to execute rustc --print sysroot");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("failed to resolve rust sysroot: {stderr}");
    }

    let sysroot =
        String::from_utf8(output.stdout).expect("rustc sysroot output is not valid UTF-8");
    PathBuf::from(sysroot.trim())
}

// 実行ファイル名をプラットフォームに応じて組み立てる
fn exe_name(base: &str) -> String {
    if std::env::consts::OS == "windows" {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

// 静的ライブラリから定義済み外部シンボルを収集する
fn collect_defined_external_symbols(nm_path: &Path, archive_path: &Path) -> BTreeSet<String> {
    let output = Command::new(nm_path)
        .arg("--defined-only")
        .arg("--extern-only")
        .arg("--format=just-symbols")
        .arg(archive_path)
        .output()
        .unwrap_or_else(|_| panic!("failed to run llvm-nm: {}", nm_path.display()));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("llvm-nm failed for {}: {stderr}", archive_path.display());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut symbols = BTreeSet::new();
    for line in stdout.lines() {
        let symbol = line.trim();
        if !is_c_identifier(symbol) {
            continue;
        }
        symbols.insert(symbol.to_string());
    }
    symbols
}

// objcopy 用の変換マップと bindgen 用の変換マップを生成する
fn build_symbol_rename_maps<F>(
    platform_symbols: BTreeSet<String>,
    rename_symbol: &F,
    is_macos: bool,
) -> SymbolRenameMaps
where
    F: Fn(&str) -> Option<String>,
{
    let mut objcopy_rename_map = BTreeMap::new();
    let mut link_name_map = BTreeMap::new();
    let mut renamed_symbols = BTreeSet::new(); // 変換後シンボルの重複を検出するためのセット

    for platform_symbol in platform_symbols {
        let is_underscore_prefixed = is_macos && platform_symbol.starts_with('_');
        // macOS ではグローバルシンボルの先頭に `_` が付くため、変換後のシンボル名を作る際には、
        // platform_symbol から先頭の '_' を外して rename_symbol を適用し、再度先頭に '_' を戻すようにする。
        // 例: `_opus_encode` -> `opus_encode` -> `shiguredo_opus_encode` -> `_shiguredo_opus_encode`
        let symbol_name = if is_underscore_prefixed {
            &platform_symbol[1..]
        } else {
            &platform_symbol
        };
        if !is_c_identifier(symbol_name) {
            continue;
        }

        // rename_symbol を適用する。
        // rename_symbol が None を返したシンボルは書き換え対象から除外する。
        let Some(renamed_symbol_name) = rename_symbol(symbol_name) else {
            continue;
        };

        let renamed_symbol = if is_underscore_prefixed {
            format!("_{renamed_symbol_name}")
        } else {
            renamed_symbol_name.clone()
        };

        if !renamed_symbols.insert(renamed_symbol.clone()) {
            panic!(
                "symbol rename collision detected: {} is renamed to {} which is already used by another symbol",
                symbol_name, renamed_symbol
            );
        }

        objcopy_rename_map.insert(platform_symbol.clone(), renamed_symbol.clone());
        link_name_map.insert(symbol_name.to_string(), renamed_symbol.clone());
    }

    SymbolRenameMaps {
        objcopy_rename_map,
        link_name_map,
    }
}

// objcopy の --redefine-syms で使うマップファイルを書き出す
fn write_objcopy_rename_map(path: &Path, objcopy_rename_map: &BTreeMap<String, String>) {
    let mut lines = String::new();
    for (symbol, renamed_symbol) in objcopy_rename_map {
        lines.push_str(symbol);
        lines.push(' ');
        lines.push_str(renamed_symbol);
        lines.push('\n');
    }
    std::fs::write(path, lines).expect("failed to write symbol rename map");
}

// 静的ライブラリのシンボルを objcopy で書き換える
fn rewrite_archive_symbols(objcopy_path: &Path, archive_path: &Path, map_path: &Path) {
    let output_archive_path = archive_path.with_extension("renamed.tmp");

    let output = Command::new(objcopy_path)
        .arg("--redefine-syms")
        .arg(map_path)
        .arg(archive_path)
        .arg(&output_archive_path)
        .output()
        .unwrap_or_else(|_| panic!("failed to run llvm-objcopy: {}", objcopy_path.display()));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "llvm-objcopy failed for {}: {stderr}",
            archive_path.display()
        );
    }

    std::fs::remove_file(archive_path).unwrap_or_else(|_| {
        panic!(
            "failed to remove original archive before replacement: {}",
            archive_path.display()
        )
    });
    std::fs::rename(&output_archive_path, archive_path).unwrap_or_else(|_| {
        panic!(
            "failed to replace archive with rewritten file: {}",
            archive_path.display()
        )
    });
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
