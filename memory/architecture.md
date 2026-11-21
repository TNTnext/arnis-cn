# 系统架构

## 入口和模式

- `src/main.rs`
  - 无参数且启用 `gui` 时启动 Tauri 窗口。
  - 有参数时进入 `run_cli`。
  - CLI 使用独立 8 MiB 栈线程，避免 Windows 上深层地形调试栈溢出。
  - `--capabilities` 在常规校验前直接输出机器可读能力描述。
- `src/gui.rs`
  - Tauri commands、任务启动、世界选择、One World、Live、缓存和预览接口。
- `src/gui/`
  - 静态 HTML、CSS、JavaScript 和 locale JSON。
  - 没有 npm/Vite/Webpack 构建层。

## 主生成流水线

`run_cli` 的主干顺序如下：

1. 解析与规范化 `Args`
2. 决定 Java、Bedrock、Luanti 或 Hytale 输出
3. 检查磁盘与区域规模
4. 并行获取 OSM、Overture、地形/土地覆盖和可选的街景与模型数据
5. 解析 OSM，投影到 XZ 方块坐标
6. 合并 Overture/Overture 建筑来源，按优先级排序
7. 应用旋转、轮廓裁剪和地表覆盖修正
8. `data_processing::generate_world_with_options`
9. 写出世界、设置出生点、构建附加数据包和 LOD

## 模块地图

### 输入与地理

- `args.rs`：Clap 参数、默认值、互斥关系和能力 JSON
- `retrieve_data.rs`：Overpass、本地 OSM、下载器与响应校验
- `osm_pbf.rs`、`osm_tiles.rs`、`arnis_tiles.rs`：PBF 与瓦片归档；中国全国和省级来源复用 PBF 下载、缓存、Data Plan、离线检查和本地裁切
- `coordinate_system/`：经纬度、笛卡尔坐标、边界框与变换
- `projection/`：本地坐标和 Web Mercator
- `clipping.rs`：几何裁剪

### 地理派生数据

- `elevation/`、`elevation_data.rs`：高程提供者、缓存、细节和峰顶
- `land_cover/`：自然地物、水体、海岸与地表覆盖
- `climate.rs`、`climate_field.rs`、`ecoregion.rs`：气候和生态区
- `canopy/`：树冠高度
- `survey/`：LiDAR、航拍和测得细节
- `overture/`：额外建筑轮廓
- `mapillary/`：街景获取、匹配、立面重建
- `models_3d/`：外部 3D 模型和体素化

### OSM 解析

- `osm_parser.rs`
  - 过滤项目不读取的标签以降低内存。
  - 将 OSM 节点、路径和关系转换为 `ProcessedElement`。
  - 处理建筑部件、内环、轮廓抑制和投影。
- `map_item.rs`、`map_item_palette.rs`、`map_transformation/`
  - 地图物品、配色、平移和旋转。

### 世界生成

- `data_processing.rs`
  - 顶层编排点 `generate_world_with_options`。
  - 对地表区域按面积稳定排序，保证更具体区域先写入。
  - 构建 tile pipeline、线程/内存预算、进度、预览和写出。
- `element_processing/`
  - 每种 OSM 语义一个处理器：建筑、道路、铁路、桥梁、水体、停车场、树木、车辆、交通灯等。
- `ground.rs`、`ground_generation.rs`、`ground_decoration.rs`
  - 地形高度、地表材质、水体和地面装饰。
- `floodfill.rs`、`floodfill_cache.rs`、`tile.rs`
  - 区域占据、共享填充结果和分块缓存。
- `caves/`、`trees/`、`structures/`、`decals/`
  - 洞穴、树木库、结构、标牌/贴花。

### 世界输出

- `world_editor/mod.rs`
  - `WorldEditor` 的公共写入与保存入口。
  - 分发 Java、Bedrock、Luanti、Hytale。
- `world_editor/java/`：Anvil 区块、区域、光照与元数据
- `world_editor/bedrock/`：Bedrock `.mcworld`
- `world_editor/luanti.rs`：`map.sqlite`
- `world_editor/hytale.rs`：实验性 Hytale 输出
- `one_world.rs`、`live/`、`distant_horizons/`、`voxy/`
  - 持久世界扩展、Live 会话和远景 LOD。

## 并发与资源

- 使用 Rayon tile pipeline，默认 CPU 上限约为 90%。
- 可用 `--threads` 和 `--ram-budget-mb` 调整。
- 共享 flood fill 结果按依赖计数释放。
- 一个 job 的多个 piece 会保留 halo，确保边界对象在相邻块中一致。
- 全局分配器是 mimalloc，针对大量区块和哈希表 churn。

## 中国 OSM 数据源

- CLI 入口是 `Args.china_national`；全国文件为 `https://download.openstreetmap.fr/extracts/asia/china-latest.osm.pbf`，约 1.785 GB。
- CLI 入口是 `Args.china_province`，由 `ChinaProvince` 枚举限定 33 个值。
- GUI 在 OSM Data Source 中提供 `China Nationwide` 和 `China Province`；省份下拉框是单独的持久化设置。
- 省级文件基址为 `https://download.openstreetmap.fr/extracts/asia/china/`，文件名形如 `{slug}-latest.osm.pbf`。
- 下载文件按 URL 进入现有 PBF 缓存；后续裁切、Data Plan、离线模式和 One World bake 均沿用原流水线。
- 全国与省级选项互斥，也与 `--file`、`--osm-pbf` 和 `--osm-pbf-url` 互斥。
- 中国来源在线下载失败时可回退默认来源；离线或显式本地文件不回退。One World 回退时会移除对应中国来源参数。

## 测试结构

- Rust 单元测试嵌在模块底部。
- `tests/offline.rs` 覆盖离线缓存行为。
- `tests/one_world_seam.rs` 覆盖 One World 接缝。
- `tests/gui/*.mjs` 是无需 npm 依赖的 Node 脚本。
- `tests/golden/` 覆盖立面、纹理、对齐、相机等回归数据。
