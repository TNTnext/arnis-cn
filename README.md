<img src="assets/git/banner.png" width="100%" alt="Banner">

# Arnis [![CI Build Status](https://github.com/louis-e/arnis/actions/workflows/ci-build.yml/badge.svg)](https://github.com/louis-e/arnis/actions) [<img alt="GitHub Release" src="https://img.shields.io/github/v/release/louis-e/arnis" />](https://github.com/louis-e/arnis/releases) [<img alt="GitHub Downloads (all assets, all releases" src="https://img.shields.io/github/downloads/louis-e/arnis/total" />](https://github.com/louis-e/arnis/releases) [![Download here](https://img.shields.io/badge/Download-here-green)](https://github.com/louis-e/arnis/releases) [![Discord](https://img.shields.io/discord/1326192999738249267?label=Discord&color=%237289da)](https://discord.gg/mA2g69Fhxq)

Arnis creates complex and accurate Minecraft Java Edition (1.21.4+), Bedrock Edition, Luanti (Minetest) and Hytale worlds that reflect real-world geography, topography, and architecture.

This free and open source project is designed to handle large-scale geographic data from the real world and generate detailed Minecraft worlds. The algorithm processes geospatial data from OpenStreetMap as well as elevation data to create an accurate Minecraft representation of terrain and architecture.
Generate your hometown, big cities, and natural landscapes with ease!

_**Want mobile generation or larger map sizes?** [MapSmith](https://arnismc.com/mapsmith/) generates worlds in your browser, no install required._

![Minecraft Preview](assets/git/preview.jpg)
<i>This GitHub page and [arnismc.com](https://arnismc.com) are the only official project websites. Do not download Arnis from any other website.</i>

## :keyboard: Usage
<img width="60%" src="assets/git/gui.png"><br>
Download the [latest release](https://github.com/louis-e/arnis/releases/) or [compile](#trophy-open-source) the project on your own.

Choose your area on the map using the rectangle tool, then simply click on <i>Start Generation</i>!
Additionally, you can customize various generation settings, such as world scale, spawn point, or building interior generation.

### China OpenStreetMap sources

In **Settings → OSM Data Source**, choose **China Nationwide** for the complete OpenStreetMap France China extract, or **China Province** to download one province-level extract. The nationwide file is about 1.7 GB, is cached after its first download, and is cut locally to the selected area. The data comes from OpenStreetMap and is available under the ODbL.

The same sources are available on the command line:

```bash
arnis --china-national --output-dir="C:/YOUR_PATH/.minecraft/saves" --bbox="min_lat,min_lng,max_lat,max_lng"
arnis --china-province=beijing --output-dir="C:/YOUR_PATH/.minecraft/saves" --bbox="min_lat,min_lng,max_lat,max_lng"
```

Existing global terrain, land-cover and building sources continue to supplement the selected OSM extract.

### Switchable map-data profiles

**Settings → Detailed Map Data** offers four cumulative packages, so the same installation can switch between a small standard download and the broadest open-data combination without editing code:

| Profile | What it adds |
| --- | --- |
| **Standard** | OpenStreetMap base, terrain, land cover, the existing standalone Additional Buildings choice and the measured/3D model pipeline. |
| **Overture Complete** | Overture buildings with available height and roof attributes, roads, railways and named places. |
| **China Enhanced** | Overture Complete plus CNBH-10m estimated building heights for buildings that have no mapped height or floor count. |
| **Maximum Open Data** | China Enhanced plus Overture water, land-use and land-cover polygons. This is the broadest built-in package. |

The profiles are additive and deterministic: OSM geometry stays authoritative, Overture fills the next layer, and CNBH supplies only a missing height. Explicit OSM/Overture heights, floor counts, LiDAR measurements and mapped 3D parts always win. A country-level or province-level OSM source can be selected independently in **OSM Data Source**. The Download Plan shows separate rows for Overture detail, CNBH heights and water/land use, including cached and missing tile counts.

CNBH-10m is the 2020 10 m China building-height dataset from Zenodo (CC BY 4.0). A county normally downloads one 2-degree GeoTIFF; a boundary crossing needs at most four. Arnis reads metadata and only the 256 x 256 TIFF blocks under building centroids, caching the files under `arnis-cnbh-cache`. Overture PMTiles layers are also read by range instead of as whole archives. `--prewarm` can prepare a county for a later `--offline` run.

VIDA Google-Microsoft-OSM Open Buildings remains available as the independent `--open-buildings-vida` layer. It is not part of the profiles because most checked Chinese records in that archive are OSM-sourced and are skipped to avoid duplicating the already selected OSM extract. It can still add non-OSM footprints and is cached separately under `arnis-open-buildings-cache`.

```bash
# Broadest open package for a county such as Taicang
arnis --china-province=jiangsu --map-data-profile=maximum \
  --output-dir="C:/YOUR_PATH/.minecraft/saves" \
  --bbox="31.42,121.08,31.48,121.18"

# The individual layers remain available to scripts
arnis --china-national --overture-detail --cnbh-heights --open-buildings-vida --overture-base \
  --output-dir="C:/YOUR_PATH/.minecraft/saves" \
  --bbox="min_lat,min_lng,max_lat,max_lng"
```

No open dataset guarantees every newly built house, private lane or shop in every Chinese village. CNBH is a 2020 estimated raster rather than a per-building survey, and VIDA supplies footprints rather than measured 3D. Arnis combines mapped heights and floor counts, CNBH estimates for gaps, ordinary height inference, measured LiDAR detail and 3D models where enabled, and recognises special observatory, planetarium and spherical/domed museum shapes. Large selections can hit each archive's tile budget; a county or city-sized selection gives the most complete result.

Attribution and licenses: OpenStreetMap data is under the [ODbL](https://www.openstreetmap.org/copyright). [CNBH-10m](https://doi.org/10.5281/zenodo.7923866) is distributed under CC BY 4.0. The optional VIDA [Google-Microsoft-OSM Open Buildings](https://source.coop/vida/google-microsoft-osm-open-buildings) China snapshot is dated 2024-08-27 and is distributed under the ODbL. Overture [buildings](https://docs.overturemaps.org/guides/buildings/), [transportation](https://docs.overturemaps.org/schema/reference/transportation/segment/) and base layers are under the ODbL. Overture [places](https://docs.overturemaps.org/schema/reference/places/place/) combines sources under permissive licenses including CDLA-Permissive-2.0, Apache-2.0 and CC0. See [Overture attribution](https://docs.overturemaps.org/attribution/) for the complete source list.

### Special building shapes

Observatories, planetariums and buildings named as spherical or domed museums receive a hemispherical roof when their data has no more precise shape. Recognition uses OSM tags such as `building=observatory`, `man_made=observatory`, `tourism=planetarium`, localized names containing terms such as `天文台`, `天文馆`, `球幕` or `observatory`, and matching Overture place categories. The special shape applies only to free-standing footprints of at least 24 m². An explicit `roof:shape`, a measured roof shape or mapped `building:part` geometry always takes precedence.

## 📚 Documentation

<img src="assets/git/documentation.png" width="100%" alt="Banner">

Full documentation is available in the [GitHub Wiki](https://github.com/louis-e/arnis/wiki/), covering topics such as generation settings, command line usage, technical explanations, FAQs and contribution guidelines.

[backgroundvid.webm](https://github.com/user-attachments/assets/420acc19-a850-418e-8397-1a45b05582ab)

## :trophy: Open Source
#### Key objectives of this project
- **Modularity**: Ensure that all components (e.g., data fetching, processing, and world generation) are cleanly separated into distinct modules for better maintainability and scalability.
- **Performance Optimization**: We aim to maintain strong performance and fast world generation.
- **Comprehensive Documentation**: Detailed in-code documentation for a clear structure and logic.
- **User-Friendly Experience**: Focus on making the project easy to use for end users.
- **Cross-Platform Support**: We want this project to run smoothly on Windows, macOS, and Linux.

#### How to contribute
This project is open source and welcomes contributions from everyone! Whether you're interested in fixing bugs, improving performance, adding new features, or enhancing documentation, your input is valuable. Simply fork the repository, make your changes, and submit a pull request. Please respect the above-mentioned key objectives. Contributions of all levels are appreciated, and your efforts help improve this tool for everyone.

Command line Build: ```cargo run --release --no-default-features -- --output-dir="C:/YOUR_PATH/.minecraft/saves" --bbox="min_lat,min_lng,max_lat,max_lng"```<br>
GUI Build: ```cargo run --release```<br>

All command line options, like the generation modes, winter and One World, are explained on the [Command Line Usage](https://github.com/louis-e/arnis/wiki/Command-Line-Usage) wiki page.

After your pull request is merged, I will take care of regularly creating update releases which will include your changes.

If you are using Nix, you can run the program directly with `nix run github:louis-e/arnis -- --output-dir=YOUR_PATH/.minecraft/saves --bbox="min_lat,min_lng,max_lat,max_lng"`

## :star: Star History

<a href="https://star-history.com/#louis-e/arnis&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=louis-e/arnis&Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=louis-e/arnis&Date&type=Date" />
   <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=louis-e/arnis&Date&type=Date" />
 </picture>
</a>

## :newspaper: Academic & Press Recognition

<img src="assets/git/recognition.png" width="100%" alt="Banner">

Arnis has been recognized in various academic and press publications after gaining more attention in December 2024.

[Building realistic Minecraft worlds with Open Data on AWS: How Arnis uses elevation datasets at scale](https://aws.amazon.com/de/blogs/publicsector/building-realistic-minecraft-worlds-with-open-data-on-aws-how-arnis-uses-elevation-datasets-at-scale/)

[Floodcraft: Game-based Interactive Learning Environment using Minecraft for Flood Mitigation and Preparedness for K-12 Education](https://www.researchgate.net/publication/384644535_Floodcraft_Game-based_Interactive_Learning_Environment_using_Minecraft_for_Flood_Mitigation_and_Preparedness_for_K-12_Education)

[Hackaday: Bringing OpenStreetMap Data into Minecraft](https://hackaday.com/2024/12/30/bringing-openstreetmap-data-into-minecraft/)

[TomsHardware: Minecraft Tool Lets You Create Scale Replicas of Real-World Locations](https://www.tomshardware.com/video-games/pc-gaming/minecraft-tool-lets-you-create-scale-replicas-of-real-world-locations-arnis-uses-geospatial-data-from-openstreetmap-to-generate-minecraft-maps)

[XDA Developers: Hometown Minecraft Map: Arnis](https://www.xda-developers.com/hometown-minecraft-map-arnis/)

Free to use press assets, including screenshots and logos, can be found [here](https://arnismc.com/presskit/).

## :lock: Code Signing Policy

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

The Windows executable is built from this repository by the [release workflow](.github/workflows/release.yml) on GitHub-hosted runners and is only signed after a maintainer approves the signing request. All roles (committer, reviewer and approver) are held by [louis-&#8288;e](https://github.com/louis-e).

Privacy policy: https://arnismc.com/privacypolicy.html

## :copyright: License Information
Copyright (c) 2022-2026 Louis Erbkamm (louis-e)

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.[^3]

The Luanti block-name mapping in `src/luanti_block_map.rs` is derived from [MC2MT](https://github.com/rollerozxa/MC2MT) by rollerozxa and is licensed under the GNU Lesser General Public License v2.1 or later. The full attribution and license header are preserved in that file.

Hytale is a trademark of Hypixel Studios. Arnis is not affiliated with or endorsed by Hypixel Studios.

Download Arnis only from the official source https://arnismc.com or https://github.com/louis-e/arnis/. Every other website providing a download and claiming to be affiliated with the project is unofficial and may be malicious.

The logo was made by @nxfx21.

NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.


[^1]: https://en.wikipedia.org/wiki/OpenStreetMap

[^2]: https://en.wikipedia.org/wiki/Arnis,_Germany

[^3]: https://github.com/louis-e/arnis/blob/main/LICENSE
