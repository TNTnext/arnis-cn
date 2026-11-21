# 当前状态

## 2026-11-20：首次拉取与记忆初始化

已完成：

- 从 `https://github.com/TNTnext/arnis-cn.git` 建立本地 Git 仓库。
- 当前分支为 `main`。
- HEAD 为 `568919c658ce787453cec8df2660b790acec4a3c`。
- 使用浅克隆、部分克隆和稀疏检出取得源码。
- 新增根目录 `AGENTS.md`。
- 新增本 `memory/` 目录及项目、架构、工作流、约束和状态文档。

当前检出配置：

```text
--depth 1
--filter=blob:none
sparse: /*
exclude: /assets/
exclude: /tests/golden/
```

本地所需但未提交的配置：

```text
http.curloptResolve=github.com:443:140.82.112.4
```

原因：机器 hosts 把 GitHub 域名指向本机；该设置只用于绕过当前 DNS/连接问题。

尚未完成或尚未验证：

- `assets/` 未完整检出。
- `tests/golden/` 未完整检出。
- 尚未运行 Rust 构建或测试。
- 本机当前 `PATH` 中没有 `node`，尚未运行 GUI JavaScript 测试。

已完成的文档检查：

- Markdown 文件均存在。
- 未发现行尾空白、制表符或零宽字符。
- 工作区只新增 `AGENTS.md` 和 `memory/`，克隆内容没有出现已跟踪文件改动。

下一步建议：

1. 修改代码前确认目标功能是否需要 `assets/`。
2. 只需理解或改纯 Rust 逻辑时，先运行无 GUI 的局部测试。
3. 运行应用或资源相关测试前执行 `git sparse-checkout add assets`。
4. Mapillary/golden 回归前执行 `git sparse-checkout add tests/golden`。

## 2026-11-20：接入中国省级 OSM 数据源

- 修改：
  - 新增 `--china-province` 和 33 个中国省级/特别区域选项。
  - 数据源使用 OpenStreetMap France 的 `https://download.openstreetmap.fr/extracts/asia/china/{slug}-latest.osm.pbf`。
  - 复用现有 PBF 下载、缓存、Data Plan、离线检查、本地裁切和 One World bake 流程。
  - 设置中的 OSM Data Source 新增 `China Province`，可在新数据源、Default、Local Archive 与 Region Download 间切换。
  - 省级选择会持久化；来源或省份切换会刷新已打开的 Data Plan。
  - 选择中国来源但未选省份时阻止生成，并显示提示。
  - 补齐 22 个 locale 的键；`zh-CN` 提供中文文案，其他语言保留英文回退。
  - 新增 `tests/gui/china-source.mjs` 和 Rust 数据源/窗口参数测试。
- 验证：
  - `cargo fmt -- --check` 通过。
  - `cargo check --all-targets --all-features` 通过。
  - `cargo test --all-targets --all-features`：3168 passed、24 ignored；集成测试 1 passed、1 ignored。
  - 32 个普通 GUI JavaScript 测试全部通过。
  - `cargo clippy --all-targets --all-features -- -D warnings` 被项目原有的 12 处 lint 阻断。
  - 允许上述既有 lint 后，完整 Clippy 以 `-D warnings` 通过，说明本次改动没有新增 Clippy 警告。
  - `git diff --check` 通过。
  - OpenStreetMap France 中国目录返回的 33 个 `*-latest.osm.pbf` 文件名与代码枚举逐一匹配。
  - 北京省级文件实测返回 HTTP 200，大小约 42 MB，支持 range 请求。
  - `https://gh-proxy.com/` 实测 raw GitHub README 返回 HTTP 200；已用代理恢复 `assets/`。
- 当前检出：
  - `assets/` 已补齐。
  - `tests/golden/` 仍由稀疏检出排除。
  - 当前本地 Git 配置中没有固定 GitHub IP 覆盖。
- 已知限制：
  - 中国省级数据源是省级 PBF，不是专门面向中国的商业 GIS API；首次下载后仍需按所选区域裁剪。
  - 中国省级来源在在线下载失败时可以回退默认来源；离线模式不会回退。
- 下一步：
  - 提交前复核 `git diff`，不要把 `target/`、缓存或生成世界加入工作树。
  - 需要 golden 回归时再执行 `git sparse-checkout add tests/golden`。

## 2026-11-21：接入中国全国 OSM 数据源

- 修改：
  - 新增 `--china-national`，下载 `https://download.openstreetmap.fr/extracts/asia/china-latest.osm.pbf`。
  - 数据来自 OpenStreetMap，许可为 ODbL；实测完整文件为 `1,785,113,869` 字节，约 1.785 GB。
  - GUI 的 OSM Data Source 新增 `China Nationwide / 中国全国数据`，可在 Default、全国、省级、Local Archive 和 Region Download 间切换。
  - 全国文件首次下载后按 URL 缓存；后续只按当前选区在本地裁切，不会直接生成整个中国。
  - 复用 Data Plan、`--prewarm`、`--offline`、One World bake 和下载失败回退路径。
  - 全国、省级和旧 PBF 来源互斥；切换来源后会移除不属于当前来源的参数。
  - 22 个 locale 补齐新键，中文界面和提示已说明全国与省级来源。
  - README 增加 GUI 切换方式和 `--china-national` / `--china-province` 命令示例。
  - 新增 Rust 数据源、数据计划和 GUI 窗口参数测试，以及 `tests/gui/china-source.mjs`。
- 验证：
  - `cargo fmt --all -- --check` 通过。
  - `cargo check --all-targets --all-features` 通过。
  - `cargo test --all-targets --all-features`：3171 passed、24 ignored、0 failed；另有集成测试 1 passed、1 ignored。
  - 中国相关 Rust 测试 6 个全部通过。
  - `tests/gui/china-source.mjs` 通过；GUI JavaScript 普通测试 32/32 通过。
  - 全国源 range GET 返回 `206`，支持分段下载；`HEAD` 请求偶发 `502`，不应作为可用性判断。
- 数据组合：
  - 本轮接入的是 OSM 全国矢量底图。
  - Arnis 已有的 ESA WorldCover 2021（10 米地表覆盖，CC BY 4.0）、Copernicus GLO-30 高程、Overture Buildings 仍作为全国适用的补充层。
  - 已调查 Overture Transportation PMTiles，但本轮没有接入；直接替换 OSM 交通语义会引入道路合并和属性映射风险。
- 已知限制：
  - 首次使用全国源需要下载约 1.7 GB，磁盘缓存和网络时间明显高于省级源。
  - 单一省份项目优先使用省级源；跨省或边界区域使用全国源。
  - OSM 中国数据完整度因地区而异，偏远地区可能仍比城市稀疏。
  - 中国来源在线下载失败时可回退默认来源；`--offline` 不会回退。
- 下一步：
  - 如需进一步补强建筑、交通或 POI，可另做 Overture 数据字段对照和合并测试，不应在没有语义验证时直接替换现有 OSM 处理。

## 2026-11-21：中国县级月度道路与地点补充层

目标：让太仓这类非一线县级区域在 OSM 底图之外，获得更新频率更高、道路和 POI 更完整的公开数据。结论是没有任何一个公开数据集能保证中国每个县的小路、新店和建筑都完整；当前采用分层组合，OSM 保留为底图和语义基准，Overture Maps 作为月度增量层。

- 新增 `--overture-detail`，默认关闭，仅 Earth 可用，并拒绝 Live 的普通设置路径。
- GUI 的 `Detailed Map Data / 详细地图数据` 可独立选择 `Standard / 标准` 或 `Overture Monthly Detail / Overture 月度详细数据`。它与 OSM Data Source 的 Default、China Nationwide、China Province、Local Archive、Region Download 选择互不覆盖。
- 新增 `src/overture/detail.rs`：
  - 读取 `transportation.pmtiles` 的道路、铁路、名称、路面、桥梁、隧道、路线编号等字段。
  - 读取 `places.pmtiles` 的名称、品牌、类别、网站、电话、邮箱、地址和置信度。
  - Overture 类别映射为 Arnis 已识别的 OSM 标签；低置信度地点和在建道路被过滤。
  - 已存在于 OSM 的道路通过 OSM 引用和空间相似度去重；平行道路不会误删。
  - 建筑内地点优先写为建筑名称和标签，其余地点成为独立命名节点。
  - 交通和地点主题可分别降级；一个档案暂时不可用时，另一个仍可加入，整体失败只回退 OSM，不中断地图生成。
- 取数限制：
  - 只读所选范围相交的 z14 切片，最多 4096 个 z14 切片。
  - 永不整包下载 Overture 行星档案；`transportation` 当前完整档案约 134.6 GB。
  - 缓存位于 `arnis-overture-cache/<release>/tiles/transportation/` 与 `.../places/`，支持 `--prewarm` 和后续 `--offline`。
  - 在线时优先尝试最新月份，避免发布列表临时失败后一直沿用上次旧月份；离线时保留已有缓存优先顺序。
- 太仓实测：
  - 范围 `31.42,121.08,31.48,121.18`。
  - 使用 Overture `2026-11-18.0`。
  - transportation 有 24 个含数据的 z14 切片；places 有 15 个。
  - 读取到 1592 个道路分段、4 个铁路分段、69 个有名地点，共 1665 个待合并元素。
  - 上述是合并前的月度层计数；最终新增数量还会扣掉 OSM 已映射道路和重复地点。
- 数据与许可：
  - OpenStreetMap：ODbL。
  - Overture transportation：ODbL。
  - Overture places：混合宽松来源，包括 CDLA-Permissive-2.0、Apache-2.0、CC0；完整署名见 `https://docs.overturemaps.org/attribution/`。
- 验证：
  - `cargo fmt --all -- --check` 通过。
  - `cargo check --all-targets --all-features` 通过。
  - 完整 Rust 测试：3178 passed、0 failed、25 ignored；集成测试 1 passed、1 ignored。
  - 33 个普通 GUI JavaScript 测试全部通过。
  - 太仓网络集成测试通过；该测试默认 ignored，可用 `cargo test --all-targets --all-features overture::detail::tests::fetches_taicang_detail_without_downloading_whole_archives -- --ignored --exact --nocapture` 重跑。
  - 修复了并行切片子任务重复传递 `--overture-detail` 导致的 Clap 参数冲突。
- 已知限制：
  - Overture Places 对小县城明显有用，但覆盖仍不均匀，不能把它描述为逐店、逐路绝对完整。
  - Overture 交通数据在部分地区仍大量来自 OSM，增量可能集中在较新发布或补充属性，而不是所有道路都新增。
  - 首次请求依赖网络和 Overture 可用性；已有月份缓存后可离线复用。
  - 只有 en-US 和 zh-CN 有本地化文案，其余语言沿用英文回退。
- 下一步：
  - 用完整中国 OSM 省级 PBF 加该月度层生成一个太仓世界，统计最终 OSM 去重后新增量。
  - 如需进一步提高实时性，应评估有正式授权和稳定 API 的商业地图来源；公开数据组合仍是当前可分发方案。

## 2026-11-21：特殊建筑穹顶识别

- 修改：
  - 新增天文台、天文馆、天象馆、球幕建筑及球形/圆顶博物馆的自动识别。
  - 识别信号包括 `building=observatory/planetarium/sphere/dome`、`building:shape`、`man_made=observatory/telescope`、`tourism=planetarium`、天文类 `museum` 标签，以及 `name`、`name:*`、`official_name`、`alt_name` 等名称中的中英文关键词。
  - Overture `places` 的 `observatory` 映射为 `man_made=observatory`，`planetarium` 映射为 `tourism=planetarium`；地点落入 OSM 建筑后，可把类别和名称传给该建筑。
  - 无显式形状的特殊建筑使用现有椭圆穹顶壳，并按较短的半轴抬高轮廓，使圆形地块接近真正的半球；普通推断穹顶仍保持扁圆轮廓。
  - 最低识别面积为 24 平方米，避免小型仪器棚或普通附属建筑被放大成穹顶。
- 优先级：
  - `roof:shape` 显式值最高。
  - LiDAR/实测 `arnis:roof:shape` 次之。
  - 特殊建筑推断只在地图没有显式或实测形状时启用。
  - `building:part` 不参与特殊建筑推断，保留已映射的 3D 体块和屋顶。
  - 被周围建筑包住的地块保持原生成器屋顶，避免穹顶穿入邻楼。
- 验证：
  - 新增单元测试覆盖标签识别、本地化名称识别、显式平屋顶优先、实测屋顶优先和 `building:part` 优先。
  - 针对特殊建筑识别的单元测试通过。
- 已知限制：
  - 该功能把可识别建筑近似为单一穹顶壳，不等同于逐建筑手工建模。
  - 名称和标签都缺失、且只有商业地图点位而没有建筑轮廓的天文馆，仍需先有建筑多边形才能生成穹顶。
  - 复杂博物馆的附加翼楼、真实球体下半部或非圆形主体仍可能被简化为建筑墙体加半球屋顶。
- 下一步：
  - 可继续为 `attraction` 或特定 Wikidata 天文馆补充专用模型，但不应替代通用标签推断。

## 追加记录格式

后续记录使用以下结构：

```text
## YYYY-MM-DD：任务摘要

- 修改：
- 验证：
- 已知限制：
- 下一步：
```
