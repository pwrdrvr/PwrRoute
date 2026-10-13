/**
 * The Pwr-family title strip with the PwrRoute mark and wordmark. Markup and
 * mark SVG are from design/PwrRoute - Name and Brief.dc.html (1e): one link
 * in, two browsers out, the route taken at full strength.
 */
export function TitleStrip() {
  return (
    <header className="strip">
      <div className="strip-gutter" />
      <div className="brand">
        <svg
          className="mark"
          viewBox="188 188 648 648"
          width="20"
          height="20"
          aria-hidden="true"
        >
          <g fill="none" stroke="currentColor" strokeWidth="56" strokeLinecap="round">
            <g opacity="0.55">
              <path d="M384 512C512 512 512 724 640 724" />
              <circle cx="724" cy="724" r="84" />
            </g>
            <path d="M384 512C512 512 512 300 640 300" />
            <circle cx="300" cy="512" r="84" />
            <circle cx="724" cy="300" r="84" />
          </g>
        </svg>
        <p className="wordmark">
          Pwr<b>Route</b>
        </p>
      </div>
      <div className="strip-fill" />
    </header>
  );
}
