export type ProxyMode = "direct" | "proxy";

export type ProxyModeStep = { enabled: boolean; save: boolean; discard: boolean; focusHost: boolean };

export function proxyModeStep(mode: ProxyMode, savedEnabled: boolean, host: string): ProxyModeStep {
  if (mode === "direct") return { enabled: false, save: savedEnabled, discard: true, focusHost: false };
  const ready = host.trim().length > 0;
  return { enabled: true, save: ready && !savedEnabled, discard: false, focusHost: !ready };
}

