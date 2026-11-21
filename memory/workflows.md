# 构建、测试与工作流

## 当前环境

- 日期：2026-11-20
- 主机为 Windows，执行器可从 Bash 访问同一个工作区。
- 当前目录：`C:\Users\Administrator\Documents\ChatGPT\Arnis-cn`
- Git 分支：`main`
- HEAD：`568919c658ce787453cec8df2660b790acec4a3c`
- HEAD 标题：`Merge pull request #1548 from louis-e/prepare-3.5.0-release`

## Git 与稀疏检出

当前不是普通完整克隆，而是：

```text
git clone --filter=blob:none --no-checkout --depth 1 --single-branch
git sparse-checkout init --no-cone
git sparse-checkout set --no-cone '/*' '!/assets/' '!/tests/golden/'
git sparse-checkout add assets
```

当前 `assets/` 已补回，`tests/golden/` 仍排除。所以当前 `git status` 干净不等于磁盘上有全部跟踪文件。

常用恢复命令：

```bash
# 补齐全部工作树
git sparse-checkout disable

# 只补齐资源
git sparse-checkout add assets

# 只补齐 golden 测试
git sparse-checkout add tests/golden
```

如需补齐当前提交无法从本地 partial clone 直接取得的 blob，Git 会按需访问 origin。

## 本机网络兼容

该机器的 hosts 文件曾把 `github.com` 指向受限地址。当前仓库已没有固定 IP 覆盖：

```bash
git config --local --get http.curloptResolve
```

GitHub 文件下载慢时，可把完整 GitHub/raw URL 前置 `https://gh-proxy.com/`。已验证：

```powershell
curl.exe -L `
  "https://gh-proxy.com/https://raw.githubusercontent.com/TNTnext/arnis-cn/main/README.md"
```

该代理只用于本机网络，不应写入项目配置。中国数据源连通性检查：

```powershell
curl.exe -L --range 0-0 -D - -o NUL `
  "https://download.openstreetmap.fr/extracts/asia/china-latest.osm.pbf"

curl.exe -L --range 0-0 -D - -o NUL `
  "https://download.openstreetmap.fr/extracts/asia/china/beijing-latest.osm.pbf"
```

2026-11-21 实测全国 URL 返回 `206 Partial Content`，文件类型为 `application/octet-stream`；`Content-Range` 给出总大小 `1,785,113,869` 字节。镜像对 `HEAD` 请求可能返回 `502`，应用使用 GET 下载，连通性检查应使用 range GET。

## Rust 构建

GUI：

```powershell
cargo run --release
```

CLI：

```powershell
cargo run --release --no-default-features -- --output-dir="C:\PATH\saves" --bbox="min_lat,min_lng,max_lat,max_lng"
```

只编译不运行：

```powershell
cargo build --release
cargo build --release --no-default-features
```

## 质量门禁

项目 CI 使用的核心命令：

```powershell
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features --release
cargo test --all-targets --all-features
```

如果 Linux 系统依赖未安装，先使用无 GUI 的局部命令：

```powershell
cargo test --no-default-features
cargo clippy --all-targets --no-default-features -- -D warnings
```

2026-11-20 的本机 Windows 工具链运行完整测试时需要临时环境兼容设置：

```powershell
$env:RUSTC_BOOTSTRAP='1'
$env:RUSTFLAGS='-Zcrate-attr=feature(cfg_select)'
cargo test --all-targets --all-features
```

这些变量只用于当前机器的工具链兼容，不写入项目配置。

当前完整严格 Clippy 会被项目原有的 12 处 lint 阻断，分布在不属于中国数据源改动的旧代码中。验证本次新增代码时可临时允许这些既有 lint：

```powershell
cargo clippy --all-targets --all-features -- -D warnings `
  -A clippy::duplicated_attributes `
  -A clippy::nonminimal_bool `
  -A clippy::manual_range_contains
```

## GUI JavaScript 测试

每个文件可独立运行：

```powershell
node tests/gui/grid-view.mjs
node tests/gui/meld-flags.mjs
node tests/gui/projects-v2.mjs
```

运行全部普通 GUI 脚本：

```powershell
Get-ChildItem tests/gui -Filter *.mjs | ForEach-Object { node $_.FullName; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } }
```

`tests/gui/window/live.mjs` 不包含在上述普通集合中；它需要已构建窗口和指定端口。

## 生成数据与副作用

以下内容默认忽略：

- `target/` 和 `target-*/`
- `terrain-tile-cache/`
- `arnis-tile-cache/`
- `parsed_osm_data.txt`
- `export.json`
- 根目录调试图片
- `gen/`、`build/`
- 生成的世界文件与日志

测试或生成前先选明确的临时目录，完成后确认没有把世界、缓存或调试产物加入 Git。
