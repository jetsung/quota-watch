# quota-watch 构建 / 开发 / 发布命令
# 用法: just [命令] [参数]（不带参数显示命令列表）

# 显示命令列表（默认）
default:
    @just --list

# 实际构建目录（尊重 CARGO_TARGET_DIR，未设置时为 target/）
target_dir := ```cargo metadata --no-deps --format-version 1 2>/dev/null | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4```

# 编译并运行（开发）
[group('build')]
dev:
    cargo run

# 编译 release 版本
[group('build')]
build:
    cargo build --release

# 快速类型检查
[group('build')]
check:
    cargo check

# 静态检查（clippy，警告即错误）
[group('quality')]
clippy:
    cargo clippy --all-targets -- -D warnings

# 运行测试
[group('quality')]
test:
    cargo test

# 格式化代码
[group('quality')]
fmt:
    cargo fmt --all

# 打包当前平台的 release 构建产物（本机原生构建，不做跨平台编译）
[group('package')]
package:
    @cargo build --release && VER=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2) && mkdir -p dist && if [ "$(uname -s)" = "Linux" ] || [ "$(uname -s)" = "Darwin" ]; then tar -cJf "dist/quota-watch-v$VER-$(uname -s | tr A-Z a-z)-$(uname -m).tar.xz" -C "{{target_dir}}/release" quota-watch; else zip -j "dist/quota-watch-v$VER-windows-$(uname -m).zip" "{{target_dir}}/release/quota-watch.exe"; fi && ls -lh dist/

# 构建 .deb 包（Linux）
[group('package')]
deb:
    @cargo build --release && cargo deb --no-build && mkdir -p dist && cp "{{target_dir}}/debian/"*.deb dist/ && ls -lh dist/*.deb

# 构建 .rpm 包（Linux）
[group('package')]
rpm:
    @cargo build --release && cargo generate-rpm && mkdir -p dist && cp "{{target_dir}}/generate-rpm/"*.rpm dist/ && ls -lh dist/*.rpm

# 安装打包工具（cargo-deb / cargo-generate-rpm，仅 Linux 打包需要）
[group('package')]
packager:
    cargo install cargo-deb cargo-generate-rpm

# 创建带 GPG 签名的 release tag 并推送到 github（触发 release.yml CI 构建全平台产物）
# 签名使用 git 配置的 user.signingkey
[group('release')]
release version:
    git tag -s -m "v{{version}}" v{{version}}
    git push github v{{version}}
