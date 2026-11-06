// Identifies where a saved API key may be sent: the provider plus the normalized
// server URL it was saved with. A key is only used when the current settings
// produce the same binding, so it can never leak to a different server.

export function keyBinding(provider, baseUrl) {
  let url = String(baseUrl ?? "").trim().replace(/\/+$/, "");
  try {
    const u = new URL(url);
    url = `${u.protocol}//${u.host.toLowerCase()}${u.pathname.replace(/\/+$/, "")}`;
  } catch {
    url = url.toLowerCase();
  }
  return `${provider}|${url}`;
}
