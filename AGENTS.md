# AGENTS.md

本文件面向在本仓库中工作的编码代理。先读这里，再按任务需要查看 `memory/` 中的详细记忆。

## 项目定位

Arnis 3.5.0 是一个 Rust 桌面应用和命令行工具，用 OpenStreetMap、高程、土地覆盖及其他地理数据生成真实地点对应的 Minecraft Java、Bedrock、Luanti 和 Hytale 世界。

- 上游项目：`https://github.com/louis-e/arnis`
- 当前克隆远端：`https://github.com/TNTnext/arnis-cn.git`
- 当前版本：`3.5.0`
- Rust edition：`2021`
- 默认特性：`gui`，基于 Tauri 2
- 许可证：Apache-2.0；部分 Luanti 映射代码另有 LGPL-2.1-or-later 来源说明

## 当前检出状态

当前工作树为了绕过这台机器上受限的 GitHub 网络，使用了浅克隆和部分克隆：

```text
branch: main
HEAD: 568919c658ce787453cec8df2660b790acec4a3c
clone mode: --depth 1 --filter=blob:none
sparse checkout: /*, 已补回 /assets/, 仍排除 /tests/golden/
```

这意味着：

- 源码、配置、文档、普通测试、GUI 资源和 `assets/` 已落盘。
- `tests/golden/` 尚未完整落盘。
- 未检出路径不会被普通 `git status` 显示为缺失。
- golden 测试需要相应资源时，先补齐稀疏路径。

补齐全部资源：

```bash
git sparse-checkout disable
```

只补齐运行资源：

```bash
git sparse-checkout add assets
```

只补齐 golden 测试：

```bash
git sparse-checkout add tests/golden
```

远程对象按需下载，首次补齐会很慢。不要在没有确认用途时无谓触发几百 MB 的资源下载。

GitHub 直连或资源下载慢时，可使用 `https://gh-proxy.com/` 加速完整 GitHub/raw 文件 URL。本机已验证 raw 文件可正常返回：

```powershell
curl.exe -L `
  "https://gh-proxy.com/https://raw.githubusercontent.com/TNTnext/arnis-cn/main/README.md"
```

该代理是网络临时措施，不是项目运行依赖，不应写入应用配置或提交到仓库。当前本地 Git 配置中已没有早先使用的固定 GitHub IP 覆盖；若再次遇到该机器的 DNS/hosts 问题，先检查：

```bash
git config --local --get http.curloptResolve
git ls-remote origin HEAD
```

## 快速命令

在 Windows 上运行 GUI：

```powershell
cargo run --release
```

运行命令行生成器：

```powershell
cargo run --release --no-default-features -- --output-dir="C:\PATH\saves" --bbox="min_lat,min_lng,max_lat,max_lng"
```

查看命令能力：

```powershell
cargo run --release --no-default-features -- --capabilities
```

常用检查：

```powershell
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

单个 GUI JavaScript 测试：

```powershell
node tests/gui/grid-view.mjs
```

GUI 测试脚本是独立的 `.mjs` 文件，没有 npm 构建步骤；每个文件顶部给出自己的运行命令。`tests/gui/window/live.mjs` 需要已构建的窗口，不属于普通静态测试集合。

## 构建与平台注意

- `Cargo.toml` 把构建脚本放在 `src/build.rs`，不要改回根目录 `build.rs`。
- Linux GUI 构建需要 GTK 3、WebKitGTK 4.1、libsoup 3 等系统包；仅做 CLI 时可用 `--no-default-features`。
- Windows release 使用 `windows` crate；GUI 进程会重新附着父控制台。
- Tauri 前端是 `src/gui/` 下的静态 HTML/CSS/JS，不需要打包前端框架。
- release profile 开启 thin LTO 和整数溢出检查。
- 部分依赖锁定到 Git revision；升级依赖时同步检查 `Cargo.lock`。

## 主数据流

1. `src/args.rs` 用 Clap 解析参数、应用默认值并校验组合。
2. `src/main.rs::run_cli` 规划输出目录和世界格式，并行获取 OSM、Overture、高程、土地覆盖等输入。
   - `--china-national` 从 OpenStreetMap France 下载中国全国 PBF，缓存后在本地裁切所选区域。
   - `--china-province` 从 OpenStreetMap France 下载中国省级 PBF，缓存后在本地裁切。
   - `--overture-detail` 是“Overture 完整数据”模式，会同时启用 `buildings`、`transportation` 和 `places`；只取与选区相交的 z14 瓦片，不下载整档。
3. `src/osm_parser.rs` 过滤标签、投影坐标、裁剪几何，并形成 `ProcessedElement`。
4. `src/ground.rs` 和 `src/ground_generation.rs` 建立地形、水体、地表和高度信息。
5. `src/building_sources.rs` 合并激光和 Overture 建筑，随后 `src/overture/detail.rs` 补充道路、铁路与具名地点，再执行 OSM 优先级排序。建筑内的地点标签写到建筑，其余成为独立地点节点。
6. `src/element_processing/` 将建筑、道路、铁路、水体、植被等 OSM 语义转换为方块。
7. `src/world_editor/` 将结果写入 Java Anvil、Bedrock、Luanti 或 Hytale 格式。
8. `src/one_world.rs`、`src/live/`、`src/datapack/`、`src/decals/` 等模块补充持久世界、联机、数据包和贴花功能。

## 关键不变量

- OSM 对象顺序会影响“谁先占用地表”；不要随意改动优先级和同面积并列规则。
- 地表的默认原则是更具体的小区域先写入，大区域后铺底。
- 分块并行必须保持跨块接缝一致，不能只在单块测试中正确。
- 随机行为应通过确定性种子保持可复现。
- Java、Bedrock、Luanti、Hytale 的 Y 范围、方块名和写入接口不同，修改生成逻辑时确认各格式的边界。
- `--offline` 只使用缓存；缺少输入时应明确失败，不能静默生成有洞的世界。
- GUI 选择中国全国数据时只发送 `--china-national`；选择省级数据时必须先选择省份。
- 中国全国、省级和旧 OSM 来源互斥；切回其他来源时不能残留 `--china-national` 或 `--china-province`。
- `--overture-detail` 不依赖保存下来的独立建筑开关：`apply_feature_needs` 和 GUI 下载计划都会自动请求 Overture 建筑；`--no-buildings` 仍优先关闭全部建筑补充。
- Overture 完整层只能按 PMTiles 范围读取，绝不能下载 `buildings`、`transportation` 或 `places` PMTiles 整档。
- 详细层失败或部分瓦片损坏时保留 OSM 底图继续生成；`--prewarm` 和 `--offline` 的完整性由缓存文件和 `net::note_uncached` 报告。
- GUI 与 CLI 共用 `Args`，新增字段通常需要同时处理参数解析、GUI 标志构造和本地化。
- GUI 文案改动应同步 `src/gui/locales/en-US.json`，并检查其他翻译是否保留占位符。
- 天文台、天象馆和球形/圆顶博物馆的自动穹顶只作为无显式形状时的推断；`roof:shape`、实测屋顶和映射的 `building:part` 必须保持更高优先级。
- 大型缓存、调试图片、生成世界、日志和临时输出不进入 Git。

## 修改与验证

- 小改优先做局部测试，提交前再跑完整格式化和目标测试。
- 修改 OSM 语义处理时，同时检查其排序、地表填充和跨块行为。
- 修改世界写出逻辑时，至少覆盖受影响的格式读取或回归测试。
- 涉及像素、镜像、光照或远景 LOD 时，注意对应 golden 测试可能被稀疏检出跳过。
- 不要提交 `target/`、世界文件、缓存、调试图片或本机网络配置。
- 完成非平凡工作后，更新 `memory/current-state.md`，把新事实写在已有结论之后，避免覆盖历史。

## 记忆文件

- `memory/README.md`：索引和更新规则
- `memory/project-overview.md`：产品目标、技术栈和能力范围
- `memory/architecture.md`：模块边界和主数据流
- `memory/workflows.md`：当前环境、构建、测试和资源恢复
- `memory/invariants.md`：修改代码时必须维护的行为约束
- `memory/current-state.md`：本次会话和最近已知状态
