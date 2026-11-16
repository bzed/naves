// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

// Test helper: a static camera and a player teleport driven by files in $profile:.
modded class MissionGameplay
{
	protected Camera m_NcCam;
	protected int m_NcSeq = -1;
	protected float m_NcTimer;

	override void OnUpdate(float timeslice)
	{
		super.OnUpdate(timeslice);
		m_NcTimer += timeslice;
		if (m_NcTimer < 0.25)
			return;
		m_NcTimer = 0;
		NcPoll();
	}

	void NcPoll()
	{
		string path = "$profile:nc_cam.txt";
		if (!FileExist(path))
			return;
		FileHandle fh = OpenFile(path, FileMode.READ);
		if (fh == 0)
			return;
		string line;
		FGets(fh, line);
		CloseFile(fh);
		TStringArray t = new TStringArray;
		line.Split(" ", t);
		if (t.Count() < 8)
			return;
		int seq = t.Get(0).ToInt();
		if (seq == m_NcSeq)
			return;
		m_NcSeq = seq;
		int mode = t.Get(1).ToInt();
		if (mode == 0)
		{
			if (m_NcCam)
			{
				m_NcCam.SetActive(false);
				GetGame().ObjectDelete(m_NcCam);
				m_NcCam = null;
			}
			NcAck(seq, "off");
			return;
		}
		float x = t.Get(2).ToFloat();
		float z = t.Get(3).ToFloat();
		float dy = t.Get(4).ToFloat();
		float yaw = t.Get(5).ToFloat();
		float pitch = t.Get(6).ToFloat();
		float fov = t.Get(7).ToFloat();
		float ry = GetGame().SurfaceRoadY(x, z);
		vector pos = Vector(x, ry + dy, z);
		if (!m_NcCam)
		{
			m_NcCam = Camera.Cast(GetGame().CreateObject("staticcamera", pos, true));
			if (!m_NcCam)
			{
				NcAck(seq, "nocamera");
				return;
			}
		}
		m_NcCam.SetPosition(pos);
		m_NcCam.SetOrientation(Vector(yaw, pitch, 0));
		m_NcCam.SetFOV(fov);
		m_NcCam.SetActive(true);
		GetGame().GetMission().GetHud().Show(false);
		NcAck(seq, "ok " + pos[0].ToString() + " " + pos[1].ToString() + " " + pos[2].ToString() + " surf=" + GetGame().SurfaceY(x, z).ToString() + " road=" + ry.ToString());
	}

	void NcAck(int seq, string what)
	{
		FileHandle fh = OpenFile("$profile:nc_ack.txt", FileMode.WRITE);
		if (fh == 0)
			return;
		FPrintln(fh, "" + seq + " " + what);
		CloseFile(fh);
	}
};

modded class MissionServer
{
	protected int m_NcSeq = -1;
	protected float m_NcTimer;

	override void OnUpdate(float timeslice)
	{
		super.OnUpdate(timeslice);
		m_NcTimer += timeslice;
		if (m_NcTimer < 0.25)
			return;
		m_NcTimer = 0;
		NcPoll();
	}

	void NcPoll()
	{
		string path = "$profile:nc_site.txt";
		if (!FileExist(path))
			return;
		FileHandle fh = OpenFile(path, FileMode.READ);
		if (fh == 0)
			return;
		string line;
		FGets(fh, line);
		CloseFile(fh);
		TStringArray t = new TStringArray;
		line.Split(" ", t);
		if (t.Count() < 3)
			return;
		int seq = t.Get(0).ToInt();
		if (seq == m_NcSeq)
			return;
		m_NcSeq = seq;
		float x = t.Get(1).ToFloat();
		float z = t.Get(2).ToFloat();
		array<Man> players = new array<Man>;
		GetGame().GetPlayers(players);
		string msg = "";
		for (int i = 0; i < players.Count(); i++)
		{
			PlayerBase p = PlayerBase.Cast(players.Get(i));
			if (!p)
				continue;
			p.SetAllowDamage(false);
			vector pos = Vector(x, GetGame().SurfaceRoadY(x, z) + 0.2, z);
			p.SetPosition(pos);
			msg = msg + " " + pos[1].ToString();
		}
		vector pp = Vector(x, GetGame().SurfaceY(x, z), z);
		array<Object> objs = new array<Object>;
		array<CargoBase> prox = new array<CargoBase>;
		GetGame().GetObjectsAtPosition(pp, 25, objs, prox);
		FileHandle pf = OpenFile("$profile:nc_probe.txt", FileMode.WRITE);
		if (pf != 0)
		{
			FPrintln(pf, "seq " + seq + " at " + pp.ToString() + " objects " + objs.Count());
			for (int k = 0; k < objs.Count() && k < 60; k++)
			{
				Object ob = objs.Get(k);
				if (ob)
					FPrintln(pf, ob.GetType() + " | " + ob.GetModelName() + " | " + ob.GetPosition().ToString() + " | " + ob.GetOrientation().ToString());
			}
			CloseFile(pf);
		}
		FileHandle ack = OpenFile("$profile:nc_site_ack.txt", FileMode.WRITE);
		if (ack != 0)
		{
			FPrintln(ack, "" + seq + " players=" + players.Count() + msg);
			CloseFile(ack);
		}
	}
};
