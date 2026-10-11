export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, init);
  if (!response.ok) {
    const body = await response
      .json()
      .catch(() => ({ error: response.statusText }));
    throw new Error(body.error ?? response.statusText);
  }
  return response.json() as Promise<T>;
}
export function jsonBody(value: unknown): RequestInit {
  return {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(value),
  };
}
export async function download(path: string, filename: string): Promise<void> {
  const response = await fetch(`/api${path}`);
  if (!response.ok) {
    const body = await response
      .json()
      .catch(() => ({ error: response.statusText }));
    throw new Error(body.error ?? response.statusText);
  }
  const url = URL.createObjectURL(await response.blob());
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
