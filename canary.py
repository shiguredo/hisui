import argparse
import re
import subprocess
from typing import Optional


# ファイルを読み込み、バージョンを更新
def update_version(file_path: str, dry_run: bool) -> Optional[str]:
    with open(file_path, "r", encoding="utf-8") as f:
        content: str = f.read()

    # 現在のバージョンを取得
    current_version_match = re.search(r'version\s*=\s*"([\d\.\w-]+)"', content)
    if not current_version_match:
        raise ValueError("Version not found or incorrect format in Cargo.toml")

    current_version: str = current_version_match.group(1)

    # バージョンが -canary.X を持っている場合の更新
    if "-canary." in current_version:
        new_content, count = re.subn(
            r'(version\s*=\s*")(\d+\.\d+\.\d+-canary\.)(\d+)',
            lambda m: f"{m.group(1)}{m.group(2)}{int(m.group(3)) + 1}",
            content,
        )
    else:
        # -canary.X がない場合、次のマイナーバージョンにして -canary.0 を追加
        new_content, count = re.subn(
            r'(version\s*=\s*")(\d+)\.(\d+)\.(\d+)',
            lambda m: f"{m.group(1)}{m.group(2)}.{int(m.group(3)) + 1}.0-canary.0",
            content,
        )

    if count == 0:
        raise ValueError("Version not found or incorrect format in Cargo.toml")

    # 新しいバージョンを確認
    new_version_match = re.search(r'version\s*=\s*"([\d\.\w-]+)"', new_content)
    if not new_version_match:
        raise ValueError("Failed to extract the new version after the update.")

    new_version: str = new_version_match.group(1)

    print(f"Current version: {current_version}")
    print(f"New version: {new_version}")
    confirmation: str = (
        input("Do you want to update the version? (Y/n): ").strip().lower()
    )

    if confirmation != "y":
        print("Version update canceled.")
        return None

    # Dry-run 時の動作
    if dry_run:
        print("Dry-run: Version would be updated to:")
        print(new_content)
    else:
        with open(file_path, "w", encoding="utf-8") as f:
            f.write(new_content)
        print(f"Version updated in Cargo.toml to {new_version}")

    return new_version


# cargo build 実行
def run_cargo_operations(dry_run: bool) -> None:
    if dry_run:
        print("Dry-run: Would run 'cargo build --release'")
    else:
        subprocess.run(["cargo", "build", "--release"], check=True)
        print("cargo build --release executed")


# git コミット、タグ、プッシュを実行
def git_commit_version(new_version: str, dry_run: bool) -> None:
    if dry_run:
        print("Dry-run: Would run 'git add Cargo.toml'")
        print(f"Dry-run: Would run '[canary] Bump version to {new_version}'")
    else:
        subprocess.run(["git", "add", "Cargo.toml"], check=True)
        subprocess.run(
            ["git", "commit", "-m", f"[canary] Bump version to {new_version}"],
            check=True,
        )
        print(f"Version bumped and committed: {new_version}")


# git コミット、タグ、プッシュを実行
def git_operations_after_build(new_version: str, dry_run: bool) -> None:
    if dry_run:
        print(f"Dry-run: Would run 'git tag {new_version}'")
        print("Dry-run: Would run 'git push'")
        print(f"Dry-run: Would run 'git push origin {new_version}'")
    else:
        subprocess.run(["git", "tag", new_version], check=True)
        subprocess.run(["git", "push"], check=True)
        subprocess.run(["git", "push", "origin", new_version], check=True)


# メイン処理
def main() -> None:
    parser = argparse.ArgumentParser(
        description="Update Cargo.toml version, run cargo build, and commit changes."
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Run in dry-run mode without making actual changes",
    )
    args = parser.parse_args()

    cargo_toml_path: str = "Cargo.toml"

    # バージョン更新
    new_version: Optional[str] = update_version(cargo_toml_path, args.dry_run)

    if not new_version:
        return  # ユーザーが確認をキャンセルした場合、処理を中断

    # バージョン更新後にまず git commit
    git_commit_version(new_version, args.dry_run)

    # cargo build 実行
    run_cargo_operations(args.dry_run)

    # ビルド後のファイルを git commit, タグ付け、プッシュ
    git_operations_after_build(new_version, args.dry_run)


if __name__ == "__main__":
    main()