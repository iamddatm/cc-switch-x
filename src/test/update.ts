import type { UpdateController } from "../hooks/useUpdate";

export function makeUpdateController(
  overrides: Partial<UpdateController> = {},
): UpdateController {
  return {
    currentVersion: "0.1.0",
    available: null,
    checking: false,
    installing: false,
    installStarted: false,
    progress: null,
    error: null,
    dismissed: false,
    hasUpdate: false,
    check: async () => {},
    install: async () => {},
    dismiss: () => {},
    resetDismiss: () => {},
    ...overrides,
  };
}
