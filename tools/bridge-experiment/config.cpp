// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

class CfgPatches {
    class naves_bridgetest {
        units[] = {};
        weapons[] = {};
        requiredVersion = 0.1;
        requiredAddons[] = {"DZ_Data", "DZ_Surfaces_Bliss"};
    };
};
class CfgWorldList {
    class bridgetest {};
};
class CfgWorlds {
    class DefaultWorld;
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
    };
    class bridgetest: CAWorld {
        access = 3;
        description = "Bridge test";
        worldName = "naves\worlds\bridgetest\bridgetest.wrp";
        class OutsideTerrain {
            satellite = "DZ\worlds\enoch\data\enoch_satout_co.paa";
            enableTerrainSynth = 1;
            class Layers {
                class Layer0 {
                    nopx = "DZ\surfaces_bliss\data\terrain\en_grass1_nopx.paa";
                    texture = "DZ\surfaces_bliss\data\terrain\en_grass1_ca.paa";
                };
            };
        };
        class Grid {
            offsetX = 0;
            offsetY = 1600;
            class Zoom1 {
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
            };
        };
        centerPosition[] = {800, 800, 100};
        clutterGrid = 1.0;
        clutterDist = 125;
        noDetailDist = 65;
        fullDetailDist = 10;
        midDetailTexture = "DZ\worlds\enoch\data\enoch_middle_mco.paa";
        terrainNormalTexture = "DZ\worlds\enoch\data\enoch_global_nohq.paa";
        heightBlendingMode = 1;
        bicubicMode = 1;
        class Names {};
    };
};
