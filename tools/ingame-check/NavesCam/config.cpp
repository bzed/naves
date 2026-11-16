// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

class CfgPatches
{
	class NavesCam
	{
		units[] = {};
		weapons[] = {};
		requiredVersion = 0.1;
		requiredAddons[] = {"DZ_Data", "DZ_Scripts"};
	};
};

class CfgMods
{
	class NavesCam
	{
		dir = "NavesCam";
		name = "NavesCam";
		type = "mod";
		author = "";
		version = "0.1";
		dependencies[] = {"Mission"};
		class defs
		{
			class missionScriptModule
			{
				value = "";
				files[] = {"NavesCam/scripts/5_Mission"};
			};
		};
	};
};
