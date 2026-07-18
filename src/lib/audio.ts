// Streams the emulator's 32768 Hz stereo samples to WebAudio, resampling to
// the AudioContext's native rate through a small ring buffer.

export class AudioPlayer {
  private ctx: AudioContext;
  private node: ScriptProcessorNode;
  private ring: Float32Array; // interleaved L/R
  private frames: number;
  private write = 0;
  private readFrac = 0;
  private read = 0;
  private ratio: number; // source frames per output frame

  constructor(srcRate: number) {
    this.ctx = new AudioContext();
    this.frames = 1 << 15; // ring capacity in stereo frames
    this.ring = new Float32Array(this.frames * 2);
    this.ratio = srcRate / this.ctx.sampleRate;
    this.node = this.ctx.createScriptProcessor(1024, 0, 2);
    this.node.onaudioprocess = (e) => this.process(e);
    this.node.connect(this.ctx.destination);
  }

  resume() {
    if (this.ctx.state === "suspended") void this.ctx.resume();
  }

  close() {
    this.node.disconnect();
    void this.ctx.close();
  }

  /** Push interleaved i16 L/R samples. */
  push(samples: Int16Array) {
    for (let i = 0; i + 1 < samples.length; i += 2) {
      const w = (this.write % this.frames) * 2;
      this.ring[w] = samples[i] / 32768;
      this.ring[w + 1] = samples[i + 1] / 32768;
      this.write++;
    }
  }

  private available(): number {
    return this.write - this.read;
  }

  private process(e: AudioProcessingEvent) {
    const outL = e.outputBuffer.getChannelData(0);
    const outR = e.outputBuffer.getChannelData(1);
    for (let i = 0; i < outL.length; i++) {
      if (this.available() < 2) {
        outL[i] = 0;
        outR[i] = 0;
        continue;
      }
      const idx = (this.read % this.frames) * 2;
      outL[i] = this.ring[idx];
      outR[i] = this.ring[idx + 1];
      this.readFrac += this.ratio;
      const step = Math.floor(this.readFrac);
      this.readFrac -= step;
      this.read += step;
    }
  }
}
