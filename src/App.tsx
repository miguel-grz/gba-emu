import { useCallback, useEffect, useRef, useState } from "react";
import { DEFAULT_KEYMAP, ensureWasm, generateThumbnail, GbaRunner } from "./lib/gba";
import { load, store } from "./lib/persist";
import {
  addGame,
  GameMeta,
  getRom,
  listGames,
  removeGame,
  setThumbnail,
} from "./lib/library";
import { Library } from "./components/Library";
import { Console } from "./components/Console";

interface Playing {
  name: string;
  rom: Uint8Array;
}

export function App() {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [games, setGames] = useState<GameMeta[]>([]);
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

  const playGame = useCallback(async (name: string) => {
    const rom = await getRom(name);
    if (!rom) {
      setError("ROM not found");
      return;
    }
    setPlaying({ name, rom });
  }, []);

  const removeOne = useCallback(
    async (name: string) => {
      await removeGame(name);
      await refreshGames();
    },
    [refreshGames],
  );

  const eject = useCallback(() => {
    setPlaying(null);
    setFps(0);
  }, []);

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
      <header className="topbar">
        <span className="logo" aria-hidden>
          ▸
        </span>
        <h1>Pocket</h1>
        <span className="subtitle">a modern Game Boy Advance emulator</span>
      </header>

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
      ) : (
        <Library
          ready={ready}
          games={games}
          busy={busy}
          error={error}
          onAdd={addRom}
          onPlay={playGame}
          onRemove={removeOne}
        />
      )}

      <footer className="credits">
        Built from scratch in Rust · CPU · PPU · APU · WebAssembly
      </footer>
    </div>
  );
}
