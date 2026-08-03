/** Stable sim ids, matching `pf_core::Sim::id()`. */
export type SimId = "iracing" | "acc" | "lmu";

/**
 * The supported sims and the folder levels each one files a setup under —
 * mirrors `pf_core::Sim::layout()`, which is the authority. Kept here so the
 * upload form knows which fields a sim actually needs without a round-trip.
 *
 * LMU (rFactor 2 heritage) is the reason this isn't a single "nests by track"
 * flag: it groups setups by **track** and has no car folder at all.
 */
export const SIM_OPTIONS: {
  id: SimId;
  label: string;
  /** Whether the sim's path has a `<car>` level. */
  car: boolean;
  /** Whether the sim needs a `<track>` folder to list the setup in-game. */
  track: boolean;
}[] = [
  { id: "iracing", label: "iRacing", car: true, track: false },
  { id: "acc", label: "Assetto Corsa Competizione", car: true, track: true },
  { id: "lmu", label: "Le Mans Ultimate", car: false, track: true },
];

const FALLBACK = { car: true, track: false };

/** Folder layout for a sim id; unknown ids degrade to the iRacing shape. */
export const simLayout = (id: string) =>
  SIM_OPTIONS.find((o) => o.id === id) ?? FALLBACK;

/** Display name for a sim id; unknown or missing ids show as-is. */
export const simLabel = (id: string | null) =>
  SIM_OPTIONS.find((o) => o.id === id)?.label ?? id ?? "";
