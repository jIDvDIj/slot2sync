export const auth = {
  login: {
    tagline: "Sync your emulators' saves, savestates and configs with the cloud.",
    permissionNote:
      "Slot2Sync <strong>does not access your personal data</strong>. It can only see and modify the files it creates in the provider you choose.",
    connecting: "Waiting for authorization in the browser…",
    connectFolder: "Connect folder",
    connect: "Connect to {{provider}}",
    providerLabel: "Storage provider",
    providerLocalFolder: "Local/network folder",
    comingSoon: "coming soon",
    folderPathLabel: "Folder path",
    folderPathPlaceholder: "e.g. D:\\Slot2Sync or \\\\server\\share",
    selectFolder: "Select folder…",
  },
  device: {
    nameLabel: "This device's name",
    namePlaceholder: "e.g. Gaming PC, Laptop",
  },
  account: {
    connected: "Connected",
    disconnect: "Disconnect",
  },
} as const;
