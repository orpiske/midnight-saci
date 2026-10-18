/** The "Midnight Saci" wordmark: Bricolage Grotesque ExtraBold with tight
 * tracking, "Midnight" in Moonlight and "Saci" in Cap Red
 * (artwork/BRAND.md section 2). The mark and icon artwork are not in the
 * repository yet (artwork/README.md), so no symbol is drawn in their place. */
export function Wordmark() {
  return (
    <span className="wordmark">
      <span className="wordmark-moon">Midnight</span> <span className="wordmark-cap">Saci</span>
    </span>
  );
}

/** The full lockup for the connection screen: wordmark and tagline. */
export function Lockup() {
  return (
    <div className="lockup">
      <Wordmark />
      <span className="overline">AI test engineer</span>
    </div>
  );
}
