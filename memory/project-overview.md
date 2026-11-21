# 项目要点

## 这是什么

Arnis 把真实地理位置转换为体素游戏世界。输入核心是 OpenStreetMap 和公开高程数据，输出支持：

- Minecraft Java Edition，Anvil 区域格式，目标版本 1.21.4+
- Minecraft Bedrock Edition，`.mcworld`
- Luanti / Minetest，`map.sqlite`
- Hytale，实验性支持

项目提供 Tauri 桌面 GUI 和完整 CLI。GUI 无独立前端构建流程，前端文件直接放在 `src/gui/`。

## 当前版本

- 包名：`arnis`
- 版本：`3.5.0`
- Rust edition：`2021`
- 默认 feature：`gui`
- 仓库许可证：Apache-2.0

`Cargo.toml` 的 repository 和 homepage 指向上游 `louis-e/arnis`。当前本地远程是 `TNTnext/arnis-cn` fork。

## 主要能力

- 按边界框、多边形或 GeoJSON 轮廓选择区域
- 从 Overpass、OSM PBF、本地 OSM 文件、Arnis tile archive、中国全国 PBF 或中国省级 PBF 获取数据
- 真实高程、土地覆盖、气候、生态区、树冠高度和部分 LiDAR 细节
- 道路、铁路、建筑、水体、桥梁、隧道、停车场、植被和大量 OSM 要素转换
- 建筑物立面、室内、洞穴、矿石、装饰和 3D 模型
- Java One World 持久扩展与分块任务
- Mapillary 街景立面分析
- Distant Horizons、Voxy、光照和 LOD 缓存
- 本地联机服务器、RCON、Mojang 查询等 `live` 功能
- 多语言 GUI

## 数据与网络

联网运行会访问多个外部数据源，区域越大、细节选项越多，下载与内存需求越高。

重要运行模式：

- 默认在线获取缺失数据。
- `--china-national` 从 OpenStreetMap France 下载完整的中国 PBF，缓存一次后在本地裁切所选区域；GUI 中对应“中国全国数据”。
- `--china-province <PROVINCE>` 从 OpenStreetMap France 下载所选中国省级 PBF，缓存一次后在本地裁切；GUI 中对应“中国省级数据”。
- `--prewarm` 只准备区域缓存，不写世界。
- `--offline` 只使用已有缓存，缺数据时停止。
- `--file` 可直接从本地 OSM 输入生成，适合可重复测试。
- `--save-json-file` 可保存本次使用的 OSM 数据。

中国全国数据源为 `https://download.openstreetmap.fr/extracts/asia/china-latest.osm.pbf`，数据来自 OpenStreetMap，许可为 ODbL，实测文件约 1.785 GB。全国 PBF 是完整国家包，不会一次生成整个国家；Arnis 仍按当前 bbox 在本地裁切。

中国省级数据覆盖 33 个省级/特别区域，包括香港、澳门、内蒙古等。`hong-kong` 和 `inner-mongolia` 是 CLI/GUI 值，下载文件名分别使用 `hong_kong` 与 `inner_mongolia`。

缓存目录、世界输出和调试图片通常不进入仓库。

## 资源

`assets/` 包含纹理、字体、树/车/船等模型、数据包、Bedrock addon 和文档图片。它们对实际运行和部分测试很重要，但体积较大。

`tests/golden/` 保存 Mapillary 立面等算法的基准数据。部分图片被 `.gitignore` 排除，测试会按可用资源跳过像素级检查。
