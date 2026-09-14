import { auth } from "./auth";
import { common } from "./common";
import { emulators } from "./emulator";
import { errors } from "./errors";
import { library } from "./library";
import { settings } from "./settings";

/** English is the default language; the shape of this object is the source of truth (`Resources`). */
export const en = {
  ...common,
  ...auth,
  ...emulators,
  ...library,
  ...settings,
  ...errors,
} as const;

export type Resources = typeof en;

export type { Localized } from "../types";
