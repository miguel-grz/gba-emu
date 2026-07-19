import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ensureWasm, generateThumbnail, GbaRunner } from "./lib/gba";
import { load, store } from "./lib/persist";
import { coverUrl, resolveTitle } from "./lib/gamedb";
import {
  addGame,
  GameMeta,
  getRom,
  listGames,
  markPlayed,
  removeGame,
  renameGame,
  setDetails,
  setThumbnail,
  toggleFavorite,
} from "./lib/library";
import { Controls, loadControls, saveControls } from "./lib/controls";
import { InputManager } from "./lib/input";
import { initDesktop } from "./lib/desktop";
import { Sidebar, Section } from "./components/Sidebar";
import { Library } from "./components/Library";
import { Console } from "./components/Console";
import { Settings } from "./components/Settings";

interface Playing {
  name: string;
  rom: Uint8Array;
}

export function App() {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [games, setGames] = useState<GameMeta[]>([]);
  const [section, setSection] = useState<Section>("library");
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [playing, setPlaying] = useState<Playing | null>(null);
  const [fps, setFps] = useState(0);
  const [flash, setFlash] = useState("");
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const runnerRef = useRef<GbaRunner | null>(null);
  const [controls, setControls] = useState<Controls>(() => loadControls());
  const inputRef = useRef<InputManager | null>(null);

  const updateControls = useCallback((next: Controls) => {
    saveControls(next);
    setControls(next);
  }, []);

  const refreshGames = useCallback(async () => {
    try {
      setGames(await listGames());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    ensureWasm().then(
      () => setReady(true),
      (e) => setError(String(e)),
    );
    refreshGames();
  }, [refreshGames]);

  const importRom = useCallback(
    async (name: string, bytes: Uint8Array) => {
      setError(null);
      setBusy(name);
      try {
        await addGame(name, bytes, {
          title: resolveTitle(bytes, name),
          cover: coverUrl(bytes),
        });
        await refreshGames();
        const thumb = await generateThumbnail(bytes);
        await setThumbnail(name, thumb);
        await refreshGames();
      } catch (e) {
        setError(String(e));
      }
      setBusy(null);
    },
    [refreshGames],
  );

  const addRom = useCallback(
    async (file: File) => {
      importRom(file.name, new Uint8Array(await file.arrayBuffer()));
    },
    [importRom],
  );

  // Native desktop menu (File > Open ROM). No-op on the web.
  useEffect(() => {
    let active = true;
    let cleanup = () => {};
    initDesktop(importRom).then((fn) => {
      if (active) cleanup = fn;
      else fn();
    });
    return () => {
      active = false;
      cleanup();
    };
  }, [importRom]);

  const renameOne = useCallback(
    async (name: string, title: string) => {
      await renameGame(name, title);
      await refreshGames();
    },
    [refreshGames],
  );

  // Backfill titles/covers for games imported before the game database existed.
  useEffect(() => {
    if (!ready) return;
    (async () => {
      const stale = games.filter((g) => g.title === undefined);
      if (stale.length === 0) return;
      for (const g of stale) {
        const rom = await getRom(g.name);
        if (!rom) continue;
        await setDetails(g.name, { title: resolveTitle(rom, g.name), cover: coverUrl(rom) });
      }
      refreshGames();
    })();
  }, [ready, games, refreshGames]);

  const playGame = useCallback(
    async (name: string) => {
      const rom = await getRom(name);
      if (!rom) {
        setError("ROM not found");
        return;
      }
      await markPlayed(name);
      setPlaying({ name, rom });
    },
    [],
  );

  const removeOne = useCallback(
    async (name: string) => {
      await removeGame(name);
      await refreshGames();
    },
    [refreshGames],
  );

  const toggleFav = useCallback(
    async (name: string, favorite: boolean) => {
      await toggleFavorite(name, favorite);
      await refreshGames();
    },
    [refreshGames],
  );

  const eject = useCallback(() => {
    setPlaying(null);
    setFps(0);
    refreshGames();
  }, [refreshGames]);

  const navigate = useCallback((s: Section) => {
    setPlaying(null);
    setSection(s);
  }, []);

  // Games shown in the current section, filtered by the search box.
  const visibleGames = useMemo(() => {
    let list = games;
    if (section === "favorites") list = list.filter((g) => g.favorite);
    else if (section === "recents")
      list = list.filter((g) => g.lastPlayed).sort((a, b) => (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0));
    const q = search.trim().toLowerCase();
    if (q)
      list = list.filter(
        (g) =>
          g.name.toLowerCase().includes(q) || (g.title ?? "").toLowerCase().includes(q),
      );
    return list;
  }, [games, section, search]);

  const counts = useMemo(
    () => ({ library: games.length, favorites: games.filter((g) => g.favorite).length }),
    [games],
  );

  // Drive the emulator whenever a game is being played.
  useEffect(() => {
    if (!playing || !canvasRef.current) return;
    let runner: GbaRunner;
    try {
      runner = new GbaRunner(playing.rom, canvasRef.current);
    } catch (e) {
      setError(String(e));
      setPlaying(null);
      return;
    }

    const batteryKey = `pocket:battery:${playing.name}`;
    const savedBattery = load(batteryKey);
    if (savedBattery) runner.loadBattery(savedBattery);

    runner.onFps = setFps;
    runner.start();
    runnerRef.current = runner;

    const persistBattery = () => store(batteryKey, runner.batteryData());
    const batteryTimer = window.setInterval(persistBattery, 5000);

    const input = new InputManager(controls, (button, pressed) => {
      runner.resumeAudio();
      runner.setButton(button, pressed);
    });
    input.attach();
    inputRef.current = input;

    return () => {
      input.detach();
      inputRef.current = null;
      window.clearInterval(batteryTimer);
      persistBattery();
      runner.stop();
      runnerRef.current = null;
    };
  }, [playing]);

  // Apply control edits to a running game without restarting it.
  useEffect(() => {
    inputRef.current?.setControls(controls);
  }, [controls]);

  const showFlash = useCallback((msg: string) => {
    setFlash(msg);
    setTimeout(() => setFlash(""), 1400);
  }, []);

  const saveState = useCallback(() => {
    const runner = runnerRef.current;
    if (!runner || !playing) return;
    try {
      store(`pocket:save:${playing.name}`, runner.saveState());
      showFlash("State saved");
    } catch (e) {
      showFlash("Save failed");
      console.error(e);
    }
  }, [playing, showFlash]);

  const loadState = useCallback(() => {
    const runner = runnerRef.current;
    if (!runner || !playing) return;
    const bytes = load(`pocket:save:${playing.name}`);
    if (!bytes) {
      showFlash("No saved state");
      return;
    }
    try {
      runner.loadState(bytes);
      showFlash("State loaded");
    } catch (e) {
      showFlash("Load failed");
      console.error(e);
    }
  }, [playing, showFlash]);

  return (
    <div className="app">
      <Sidebar active={section} counts={counts} onSelect={navigate} />

      <main className="main">
        {playing ? (
          <Console
            canvasRef={canvasRef}
            fileName={playing.name}
            fps={fps}
            flash={flash}
            onEject={eject}
            onSave={saveState}
            onLoad={loadState}
          />
        ) : section === "settings" ? (
          <Settings controls={controls} onControlsChange={updateControls} />
        ) : (
          <Library
            section={section}
            ready={ready}
            games={visibleGames}
            busy={busy}
            error={error}
            search={search}
            onSearch={setSearch}
            onAdd={addRom}
            onPlay={playGame}
            onToggleFav={toggleFav}
            onRemove={removeOne}
            onRename={renameOne}
          />
        )}
      </main>
    </div>
  );
}
