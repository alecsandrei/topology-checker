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
  // The server supplies Content-Disposition. Let the browser stream directly to
  // its download manager instead of allocating the entire response as a Blob.
  const link = document.createElement("a");
  const url = new URL(`/api${path}`, window.location.origin);
  url.searchParams.set("download", filename);
  link.href = url.toString();
  link.download = filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
}
