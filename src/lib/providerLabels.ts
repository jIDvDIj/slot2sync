import type { TFunction } from "i18next";

import type { ProviderKind } from "../types/ipc";

/** Brand names are not translated. */
export function providerLabel(provider: ProviderKind, t: TFunction): string {
  switch (provider) {
    case "google_drive":
      return "Google Drive";
    case "dropbox":
      return "Dropbox";
    case "one_drive":
      return "OneDrive";
    case "local_folder":
      return t("providers.localFolder");
  }
}
