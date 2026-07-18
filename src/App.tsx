import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { DEFAULT_KEYMAP, ensureWasm, generateThumbnail, GbaRunner } from "./lib/gba";
import { load, store } from "./lib/persist";
import {
  addGame,
  GameMeta,
  getRom,
  listGames,
  markPlayed,
  removeGame,
  setThumbnail,
  toggleFavorite,
} from "./lib/library";
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

  const addRom = useCallback(
    async (file: File) => {
      setError(null);
      const bytes = new Uint8Array(await file.arrayBuffer());
      setBusy(file.name);
      try {
        await addGame(file.name, bytes);
        await refreshGames();
        const thumb = await generateThumbnail(bytes);
        await setThumbnail(file.name, thumb);
        await refreshGames();
      } catch (e) {
        setError(String(e));
      }
      setBusy(null);
    },
    [refreshGames],
  );

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
    if (q) list = list.filter((g) => g.name.toLowerCase().includes(q));
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

    const onKey = (pressed: boolean) => (e: KeyboardEvent) => {
      const button = DEFAULT_KEYMAP[e.code];
      if (button === undefined) return;
      e.preventDefault();
      runner.resumeAudio();
      runner.setButton(button, pressed);
    };
    const down = onKey(true);
    const up = onKey(false);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);

    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.clearInterval(batteryTimer);
      persistBattery();
      runner.stop();
      runnerRef.current = null;
    };
  }, [playing]);

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
          <Settings />
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
          />
        )}
      </main>
    </div>
  );
}
