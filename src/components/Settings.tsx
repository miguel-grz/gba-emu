export function Settings() {
  return (
    <section className="settings view">
      <div className="hero" style={{ minHeight: 140 }}>
        <div style={{ position: "relative", zIndex: 2 }}>
          <div className="hero__eyebrow">Pocket</div>
          <h1 className="hero__title">Settings</h1>
        </div>
      </div>

      <div className="panel">
        <h3>Controls</h3>
        {[
          ["D-Pad", "Arrow keys"],
          ["A / B", "X / Z"],
          ["L / R", "A / S"],
          ["Start / Select", "Enter / Backspace"],
        ].map(([a, b]) => (
          <div className="row" key={a}>
            <span>{a}</span>
            <kbd>{b}</kbd>
          </div>
        ))}
        <div className="row">
          <span>Remapping &amp; gamepad</span>
          <span style={{ color: "var(--violet-bright)" }}>Coming soon</span>
        </div>
      </div>

      <div className="panel">
        <h3>About</h3>
        <div className="row">
          <span>Emulator core</span>
          <span>Rust → WebAssembly</span>
        </div>
        <div className="row">
          <span>Your games</span>
          <span>Stored in this browser</span>
        </div>
        <div className="row">
          <span>BIOS</span>
          <span>High-level (none required)</span>
        </div>
      </div>
    </section>
  );
}
