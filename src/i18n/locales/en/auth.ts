export const auth = {
  login: {
    tagline: "Sync your emulators' saves, savestates and configs with Google Drive.",
    permissionNote:
      "Slot2Sync <strong>does not access your personal data</strong>. It can only see and modify the files it creates in your Google Drive.",
    connecting: "Waiting for authorization in the browser…",
    connect: "Connect to Google Drive",
  },
  device: {
    nameLabel: "This device's name",
    namePlaceholder: "e.g. Gaming PC, Laptop",
  },
  account: {
    connected: "Google account connected",
    disconnect: "Disconnect",
  },
} as const;
