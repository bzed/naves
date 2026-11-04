// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The world's `config.cpp`, generated as text.
//!
//! The field list is what `tools/bridge-experiment/config.cpp` needed for binarize and a 1.30 server to
//! load a world, see `docs/formats/binarize-world.md`. The text contains no Bohemia content, only paths
//! that refer to files of the game.

use naves_core::heightmap::Heightmap;
use naves_core::ident::ascii_name;
use naves_core::project::Project;

/// The forward declarations the game's own world configs carry for `CAWorld`.
const CAWORLD_DECLARATION: &str = r"    class DefaultWorld;
    class CAWorld: DefaultWorld {
        class Weather {
            class Overcast {
                class Weather1;
                class Weather2;
                class Weather3;
                class Weather4;
                class Weather5;
                class Weather6;
                class Weather7;
                class Weather8;
                class Weather9;
                class Weather10;
                class Weather11;
                class Weather12;
            };
        };
    };";

/// The map grid zoom levels of the in-game map.
const GRID_ZOOMS: &str = r#"            class Zoom1 {
                zoomMax = 0.15;
                format = "XY";
                formatX = "000";
                formatY = "000";
                stepX = 100;
                stepY = -100;
            };
            class Zoom2 {
                zoomMax = 0.85;
                format = "XY";
                formatX = "00";
                formatY = "00";
                stepX = 1000;
                stepY = -1000;
            };
            class Zoom3 {
                zoomMax = 1e30;
                format = "XY";
                formatX = "0";
                formatY = "0";
                stepX = 10000;
                stepY = -10000;
            };"#;

/// The in-game path prefix of a world: `naves\worlds\<world>`.
#[must_use]
pub fn prefix(world: &str) -> String {
    format!("naves\\worlds\\{world}")
}

/// The `config.cpp` text of the project's world; `hm` supplies the highest point for `centerPosition`.
#[must_use]
pub fn world_config(project: &Project, hm: &Heightmap) -> String {
    let world = &project.world;
    let map = project.map_size_m();
    let half = map / 2.0;
    let top = hm.min_max().1;
    let shown = ascii_name(&project.name);
    let shown = if shown.is_empty() {
        world.as_str()
    } else {
        shown.as_str()
    };
    let path = prefix(world);
    format!(
        r#"class CfgPatches {{
    class naves_{world} {{
        units[] = {{}};
        weapons[] = {{}};
        requiredVersion = 0.1;
        requiredAddons[] = {{"DZ_Data", "DZ_Surfaces_Bliss"}};
    }};
}};
class CfgWorldList {{
    class {world} {{}};
}};
class CfgWorlds {{
{CAWORLD_DECLARATION}
    class {world}: CAWorld {{
        access = 3;
        description = "{shown}";
        worldName = "{path}\{world}.wrp";
        class OutsideTerrain {{
            satellite = "DZ\worlds\enoch\data\enoch_satout_co.paa";
            enableTerrainSynth = 1;
            class Layers {{
                class Layer0 {{
                    nopx = "DZ\surfaces_bliss\data\terrain\en_grass1_nopx.paa";
                    texture = "DZ\surfaces_bliss\data\terrain\en_grass1_ca.paa";
                }};
            }};
        }};
        class Grid {{
            offsetX = 0;
            offsetY = {map};
{GRID_ZOOMS}
        }};
        centerPosition[] = {{{half}, {half}, {top}}};
        clutterGrid = 1.0;
        clutterDist = 125;
        noDetailDist = 65;
        fullDetailDist = 10;
        midDetailTexture = "DZ\worlds\enoch\data\enoch_middle_mco.paa";
        terrainNormalTexture = "DZ\worlds\enoch\data\enoch_global_nohq.paa";
        heightBlendingMode = 1;
        bicubicMode = 1;
        class Names {{
            class Center {{
                name = "{shown}";
                position[] = {{{half}, {half}}};
                type = "Village";
            }};
        }};
    }};
}};
"#
    )
}
