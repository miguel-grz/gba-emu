import { useCallback, useEffect, useRef, useState } from "react";
import { DEFAULT_KEYMAP, ensureWasm, GbaRunner } from "./lib/gba";
import { Landing } from "./components/Landing";
import { Console } from "./components/Console";

export function App() {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rom, setRom] = useState<Uint8Array | null>(null);
  const [fileName, setFileName] = useState("");
  const [fps, setFps] = useState(0);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    ensureWasm().then(
      () => setReady(true),
      (e) => setError(String(e)),
    );
  }, []);

  const loadFile = useCallback(async (file: File) => {
    setError(null);
    const bytes = new Uint8Array(await file.arrayBuffer());
    setFileName(file.name);
    setRom(bytes);
  }, []);

  const eject = useCallback(() => {
    setRom(null);
    setFileName("");
    setFps(0);
  }, []);

  // Start the emulator once a ROM is loaded and the canvas is on screen.
  useEffect(() => {
    if (!rom || !canvasRef.current) return;
    let runner: GbaRunner;
    try {
      runner = new GbaRunner(rom, canvasRef.current);
    } catch (e) {
      setError(String(e));
      setRom(null);
      return;
    }
    runner.onFps = setFps;
    runner.start();

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
      runner.stop();
    };
  }, [rom]);

  return (
    <div className="app">
      <header className="topbar">
        <span className="logo" aria-hidden>
          ▸
        </span>
        <h1>Pocket</h1>
        <span className="subtitle">a modern Game Boy Advance emulator</span>
      </header>

      {rom ? (
        <Console
          canvasRef={canvasRef}
          fileName={fileName}
          fps={fps}
          onEject={eject}
        />
      ) : (
        <Landing ready={ready} error={error} onLoad={loadFile} />
      )}

      <footer className="credits">
        Built from scratch in Rust · CPU · PPU · APU · WebAssembly
      </footer>
    </div>
  );
}
