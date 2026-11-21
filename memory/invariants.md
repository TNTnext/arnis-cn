# 关键行为约束

## 地表覆盖优先级

地表方块通常只在“尚未被占用”时写入。为了得到正确结果，项目把会铺地表的区域按投影面积从小到大稳定排序：

- 更具体的小区域先写。
- 大区域后写并填充周围。
- 面积相等时用元素 ID 保证确定性。
- 排序只重排原本属于地表填充的槽位，其他元素保持原索引。

修改相关分类时必须同时检查 `way_ground_fill_area`、`relation_ground_fill_area` 和实际处理分支，避免排序分类与渲染分类不一致。

## 确定性与随机

- 同一输入、参数和版本应生成稳定结果。
- 随机细节使用项目内确定性 RNG 和种子。
- 新增并行随机逻辑时，不能使用每线程独立且不可复现的默认随机源。

## 分块与接缝

- tile 会独立处理，但共享全局顺序和区域覆盖。
- job piece 带有 halo，只写自己的区域，同时保留邻近对象对边界的影响。
- 水面、道路、桥梁、树木、洞穴和装饰都可能在接缝处出现问题。
- 修改 flood fill、共享缓存或对象跨越 tile 的行为后，应运行 `tests/one_world_seam.rs` 及相关单元测试。

## 离线模式

- `--offline` 不允许发出数据请求。
- 缺少缓存时应报告全部缺失来源并失败。
- `--prewarm` 与 `--offline` 有互斥语义；准备缓存的任务不能同时要求完全离线。
- 3D 模型、街景和外部瓦片都可能让一个看似完整的离线缓存出现额外缺口。

## 世界格式

- Java、Bedrock、Luanti、Hytale 使用不同的方块 ID、NBT、Y 范围和元数据。
- 扩展高度、洞穴和最底层修改必须同时考虑各格式的上下界。
- 写出失败要保留用户可理解的错误；磁盘已满有专门识别逻辑。

## GUI 与 CLI 同步

GUI 最终会拼接 CLI flags。新增选项通常需要同步：

1. `src/args.rs`
2. `src/gui/index.html`
3. `src/gui/js/main.js` 或相关模块
4. `src/gui/js/meld-features.js` 的 job flags
5. `src/gui/locales/en-US.json`
6. 需要时更新其他 locale 和 `tests/gui/*.mjs`

布尔参数的默认值很重要：默认设置通常不应产生额外 CLI flag。

中国 OSM 来源额外要求：

- 选择 `China Nationwide` 时只发送 `--china-national`，不能附带省份或旧 PBF 参数。
- 选择 `China Province` 后必须选择具体省份，未选择时 GUI 阻止启动。
- 省份作为 `--china-province=<value>` 发送，并与 `--file`、`--osm-pbf`、`--osm-pbf-url` 互斥。
- 全国、省份和旧 PBF 来源互斥；切回 Default、Local Archive 或 Region Download 后不能残留中国来源参数。
- 来源或省份变化后，已打开的 Data Plan 需要刷新。

## 本地化

- locale 使用带占位符的 JSON。
- 翻译时保留 `{name}` 形式的占位符。
- 修改复数文本时检查 `_one`、`_few`、`_other` 等语言特定键。
- 只改 en-US 会留下回退文本，但界面完整发布前应检查所有可见 locale。

## 资源与许可证

- `assets/` 中有第三方纹理、字体、模型和 provenance 信息。
- 替换资源前检查同目录的 `PROVENANCE.md`、`NOTICE` 或文件头。
- `src/luanti_block_map.rs` 的映射有单独 LGPL 来源说明，不要移除此许可头。
- 项目商标和 Minecraft 免责声明不应被误删。

## 性能

- tile 处理是主要热点，避免无界克隆大型元素集合。
- 尽量使用引用、紧凑 ID 和已有 flood-fill 缓存。
- 新并行阶段要设内存/线程预算，并保证取消和 panic 时能释放共享依赖。
- 性能改动应使用相同区域、相同参数前后对比；`--benchmark` 会输出分阶段计时。
