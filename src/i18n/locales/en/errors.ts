export const errors = {
  errors: {
    io: "I/O error",
    database: "Database error",
    network: "Network error",
    keyring: "Credentials vault error",
    serialization: "Serialization error",
    auth: "Authentication error",
    emulator_not_detected: "Emulator not recognized in folder",
    emulator_exists: "An emulator with this name already exists",
    file_busy: "File in use (modified while reading)",
    remote_not_found: "Folder or file not found on the remote provider",
    insufficient_disk_space: "Not enough disk space for the download",
    integrity: "Transfer integrity check failed",
    unexpected: "Unexpected error talking to the backend",
  },
} as const;
