import init, { Emulator } from "../../web/pkg/gba_web.js";
import wasmUrl from "../../web/pkg/gba_web_bg.wasm?url";
import { AudioPlayer } from "./audio";

let wasmReady: Promise<void> | null = null;

/** Load the WASM module once. */
export function ensureWasm(): Promise<void> {
  if (!wasmReady) wasmReady = init(wasmUrl).then(() => undefined);
  return wasmReady;
}

/** Button index in the GBA KEYINPUT register. */
export const Button = {
  A: 0,
  B: 1,
  Select: 2,
  Start: 3,
  Right: 4,
  Left: 5,
  Up: 6,
  Down: 7,
  R: 8,
  L: 9,
} as const;
export type Button = (typeof Button)[keyof typeof Button];

/** Default keyboard → GBA mapping. */
export const DEFAULT_KEYMAP: Record<string, Button> = {
  KeyX: Button.A,
  KeyZ: Button.B,
  Backspace: Button.Select,
  Enter: Button.Start,
  ArrowRight: Button.Right,
  ArrowLeft: Button.Left,
  ArrowUp: Button.Up,
  ArrowDown: Button.Down,
  KeyS: Button.R,
  KeyA: Button.L,
};

/** Drives one loaded cartridge: the frame loop, video, input and audio. */
export class GbaRunner {
  private emu: Emulator;
  private ctx: CanvasRenderingContext2D;
  private image: ImageData;
  private audio: AudioPlayer | null = null;
  private raf = 0;
  private keys = 0;
  private lastFpsTime = 0;
  private frameCount = 0;
  onFps: ((fps: number) => void) | null = null;

  constructor(rom: Uint8Array, canvas: HTMLCanvasElement) {
    this.emu = new Emulator(rom);
    const w = Emulator.width();
    const h = Emulator.height();
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) throw new Error("2D canvas context unavailable");
    this.ctx = ctx;
    this.image = ctx.createImageData(w, h);
  }

  start() {
    try {
      this.audio = new AudioPlayer(Emulator.sample_rate());
    } catch {
      this.audio = null; // audio is best-effort; video still runs
    }
    this.raf = requestAnimationFrame(this.loop);
  }

  stop() {
    cancelAnimationFrame(this.raf);
    this.audio?.close();
    this.audio = null;
  }

  /** Resume audio (must be called from a user gesture in most browsers). */
  resumeAudio() {
    this.audio?.resume();
  }

  setButton(button: Button, pressed: boolean) {
    if (pressed) this.keys |= 1 << button;
    else this.keys &= ~(1 << button);
    this.emu.set_keys(this.keys);
  }

  saveState(): Uint8Array {
    return this.emu.save_state();
  }

  loadState(bytes: Uint8Array) {
    this.emu.load_state(bytes);
  }

  /** The cartridge battery save (the game's .sav). */
  batteryData(): Uint8Array {
    return this.emu.save_data();
  }

  loadBattery(bytes: Uint8Array) {
    this.emu.load_save_data(bytes);
  }

  private loop = (time: number) => {
    this.emu.run_frame();
    this.image.data.set(this.emu.frame());
    this.ctx.putImageData(this.image, 0, 0);
    this.audio?.push(this.emu.drain_samples());

    this.frameCount++;
    if (time - this.lastFpsTime >= 500) {
      const fps = (this.frameCount * 1000) / (time - this.lastFpsTime);
      this.onFps?.(Math.round(fps));
      this.frameCount = 0;
      this.lastFpsTime = time;
    }
    this.raf = requestAnimationFrame(this.loop);
  };
}
