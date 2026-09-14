import type { Localized, Resources } from "../en/index";
import { auth } from "./auth";
import { common } from "./common";
import { emulators } from "./emulator";
import { errors } from "./errors";
import { library } from "./library";
import { settings } from "./settings";

/** Brazilian Portuguese. */
export const pt: Localized<Resources> = {
  ...common,
  ...auth,
  ...emulators,
  ...library,
  ...settings,
  ...errors,
};
