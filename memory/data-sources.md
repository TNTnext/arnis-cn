# 地图与 3D 数据源

本文件记录 Arnis-cn 当前可切换的开放数据组合、缓存和限制。它只记录已经接入或有明确结论的来源。

## 设置中的四档

| 档位 | CLI | 包含 |
| --- | --- | --- |
| 标准数据 | `--map-data-profile=standard` | OSM 底图、地形、ESA WorldCover 土地覆盖、独立建筑补充和已有 3D 模型/实测流程 |
| Overture 完整数据 | `--map-data-profile=overture` | 标准 + Overture buildings、transportation、places |
| 中国增强 | `--map-data-profile=china-plus` | Overture 完整数据 + CNBH-10m 建筑高度 |
| 最高完整度 | `--map-data-profile=maximum` | 中国增强 + Overture base water、land_use、land_cover |

单独参数：

- `--overture-detail`：Overture 建筑、道路、铁路和地点。
- `--cnbh-heights`：给中国建筑中缺少 `height` 和 `building:levels` 的对象补 2020 年估算高度。
- `--open-buildings-vida`：VIDA 中国开放建筑轮廓。
- `--overture-base`：Overture 水体、土地利用和土地覆盖。

四档是累计组合。VIDA 是可选的独立轮廓层，不包含在四档中，因为抽查的中国记录大量来自 OSM，重复加入会增加请求和去重成本。`--no-buildings` 会关闭 OSM/Overture/VIDA 的建筑补充以及 CNBH，但 Overture 道路、地点和 base 水土表面仍可保留。

## 来源明细

### OpenStreetMap 底图

- 默认来源：Overpass、OSM PBF 或现有 tile archive，取决于 CLI/GUI 选择。
- 中国全国：`https://download.openstreetmap.fr/extracts/asia/china-latest.osm.pbf`
- 中国省级：`https://download.openstreetmap.fr/extracts/asia/china/{slug}-latest.osm.pbf`
- 许可：ODbL。
- OSM 是当前语义基准和最高优先级。它决定已映射建筑、道路、水体、铁路和地点的本地几何。

### Overture 月度详细层

- 主题：`buildings`、`transportation`、`places`。
- 读取：PMTiles z14，仅请求与 bbox 相交的瓦片。
- 缓存：`arnis-overture-cache/<release>/tiles/<theme>/`。
- 建筑属性可含高度、楼层和屋顶信息；缺失时交给 Arnis 现有高度推断。
- 交通和地点按 OSM 引用及空间相似度去重。
- 许可：建筑和交通 ODbL；地点为混合宽松许可，完整署名见 Overture attribution。

### VIDA China Open Buildings

- 名称：VIDA Google-Microsoft-OSM Open Buildings。
- URL：`https://data.source.coop/vida/google-microsoft-osm-open-buildings/pmtiles/by_country/country_iso=CHN/CHN.pmtiles`
- 快照：`CHN-2024-08-27`。
- 实测文件大小：`699,757,429` 字节，支持 HTTP range。
- 图层：`CHN`；读取 z14；最多 4096 个相交瓦片，最多 1,500,000 个轮廓。
- 使用 MVT `Feature.id` 作为稳定轮廓 ID，跳过 `bf_source=osm`。
- 缓存：`arnis-open-buildings-cache/CHN-2024-08-27/tiles/`。
- 许可：ODbL。
- 限制：主要提供二维建筑轮廓，没有统一逐栋高度。它只补 OSM/Overture 都没有建筑的空间，之后的墙体高度仍按现有规则推断。

### CNBH-10m 中国建筑高度

- 名称：CNBH-10m，China Building Height 10 m prediction。
- 年份：2020 年估算产品。
- DOI：`10.5281/zenodo.7923866`。
- URL：`https://zenodo.org/api/records/7923866/files/CNBH10m_X{X}Y{Y}.tif/content`。
- 许可：CC BY 4.0。
- 分片：每个 GeoTIFF 覆盖 2°×2°，实际文件坐标使用奇数索引。例如 `X121Y31` 覆盖约 `120..122 E, 30..32 N`，太仓选它。
- 文件：GeoTIFF，Float32，10 米像素；实测 `X101Y31` 为 `19496×22517` 像素、UTM 47N，`X121Y31` 为 UTM 51N。
- 读取方式：读取 GeoKey、PixelScale 和 Tiepoint 后，把建筑质心投影到文件 UTM CRS，只解码包含质心的 256×256 TIFF 块；每点取 3×3 中值并过滤无效值和极端值。
- 优先级：仅作用于没有 `height`、没有 `building:levels`、不是 `building:part` 的建筑。OSM/Overture 显式属性、VIDA 之后的实测 LiDAR 和外部 3D 模型均优先。
- 标记：写入 `arnis:height_source=cnbh10m_2020`。
- 缓存：`arnis-cnbh-cache/CNBH10m-2020/`，支持 `.tif.part` 断点续传。
- 范围：县城通常 1 个约数百 MB 文件，跨边界最多 4 个；下载计划在未缓存时按约 500 MB/文件估算。
- 限制：这是 10 米格网的高度预测，不是逐栋实测；2020 年后新建建筑、屋顶附属物和复杂地标的高度仍可能偏差。

### Overture base 水土表面

- URL 模式：`https://overturemaps-extras-us-west-2.s3.us-west-2.amazonaws.com/tiles/<release>/base.pmtiles`
- 主题：`water`、`land_use`、`land_cover`。
- 读取：z13，仅请求与 bbox 相交的瓦片；最多 4096 个瓦片、1,000,000 个 feature。
- 跳过 OSM 来源几何；只保留经过白名单验证的 OSM 兼容标签。
- 缓存：`arnis-overture-cache/<release>/tiles/base/`。
- 许可：Overture base，ODbL。
- 用途：水库、运河、公园、工业区、保护地、湿地、农田和自然覆盖等。

### 高程、地表和生态

- Copernicus/Mapterhorn 等现有高程提供者。
- ESA WorldCover 2021 土地覆盖，CC BY 4.0。
- 现有气候、生态区、树冠高度和 LiDAR/测量细节流程。
- 这些来源与四档地图数据独立，仍按原有参数和范围限制运行。

## 3D 与特殊建筑

当前 3D 不是来自单一“全球逐栋 3D 库”，而是组合：

1. OSM/Overture 的 `height`、`building:levels`、`min_height`、`roof:shape` 等属性。
2. Arnis 对缺失高度的普通建筑，按建筑面积和类型推断。
3. LiDAR/实测高度、屋顶形状和花园细节。
4. `models_3d/` 的外部地标模型流水线。
5. 天文台、天文馆、球幕建筑、球形/圆顶博物馆的自动穹顶推断。

特殊建筑的优先级：

1. 显式 `roof:shape`。
2. 实测或 LiDAR `arnis:roof:shape`。
3. 名称/类别推断的穹顶。

`building:part` 已有体块时不套用自动穹顶，避免破坏映射的复杂建筑。

## 3D-GloBFP 调查结论

- 3D-GloBFP 提供全球建筑轮廓与高度，Zenodo 记录可访问，许可证为 CC BY 4.0。
- 实际省级文件由 Figshare 托管；2026-11-21 在本机从 Figshare API、文章页和 `ndownloader` 均得到 HTTP 403。
- 当前没有宣称已接入全中国逐栋 3D-GloBFP。若以后找到稳定可访问镜像，应先做省/县分区、格式解析和与 OSM/Overture 的空间去重，再作为可选 3D 高度源接入。

## 完整性边界

- 没有任何公开数据集能保证中国每个自然村、新建小区、私宅和临时建筑逐栋、逐路、逐店、逐日完整。
- OSM 对中国县城的覆盖不一致；Overture 是更广的月度聚合；VIDA 可作为额外乡镇轮廓补充，但抽查中大量记录来自 OSM。
- CNBH 可补 2020 年建筑高度空缺，但不能替代 2020 年后的新建筑、道路、店铺或精细 3D 几何。
- Overture base 可补水体、土地用途和地表语义，不替代 OSM 的道路/建筑编辑语义。
- 大范围请求会触及瓦片预算或内存上限。县城/城市范围比省或全国范围更完整，首次使用建议先打开“下载计划”。
