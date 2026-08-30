type OrganicLoaderProps = {
  variant?: "dots" | "nucleus";
  size?: number;
  className?: string;
  label?: string;
};

// ponytail: fixed filter id, duplicate ids on a page resolve to the same filter
export function OrganicLoader({
  variant = "nucleus",
  size = 80,
  className = "",
  label = "Loading",
}: OrganicLoaderProps) {
  return (
    <svg
      role="status"
      aria-label={label}
      width={size}
      height={size}
      viewBox="0 0 200 200"
      className={className}
      style={{ overflow: "visible" }}
    >
      <defs>
        <filter id="pf-loader-goo">
          <feGaussianBlur in="SourceGraphic" stdDeviation="6" result="blur" />
          <feColorMatrix
            in="blur"
            mode="matrix"
            values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 22 -11"
            result="goo"
          />
          <feComposite in="SourceGraphic" in2="goo" operator="atop" />
        </filter>
      </defs>

      <g style={{ filter: "url(#pf-loader-goo)" }} fill="currentColor">
        {variant === "dots" ? (
          <>
            <circle className="lo-dot" cx="60" cy="100" r="20" style={{ animationDelay: "0s" }} />
            <circle
              className="lo-dot"
              cx="100"
              cy="100"
              r="20"
              style={{ animationDelay: "-0.2s" }}
            />
            <circle
              className="lo-dot"
              cx="140"
              cy="100"
              r="20"
              style={{ animationDelay: "-0.4s" }}
            />
          </>
        ) : (
          <>
            <circle className="lo-nuc" cx="100" cy="100" r="24" />
            <circle className="lo-sat" cx="100" cy="100" r="12" style={{ animationDelay: "0s" }} />
            <circle
              className="lo-sat"
              cx="100"
              cy="100"
              r="12"
              style={{ animationDelay: "-0.65s" }}
            />
            <circle
              className="lo-sat"
              cx="100"
              cy="100"
              r="12"
              style={{ animationDelay: "-1.3s" }}
            />
            <circle
              className="lo-sat"
              cx="100"
              cy="100"
              r="12"
              style={{ animationDelay: "-1.95s" }}
            />
          </>
        )}
      </g>
    </svg>
  );
}
