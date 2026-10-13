// Imported by stress.ts so the spike's module loader is exercised.
export const WORK_DOMAINS: string[] = ["corp.example.com", "jira.example.com"];

export function stripTracking(url: URL): URL {
  for (const key of [...url.searchParams.keys()]) {
    if (key.startsWith("utm_") || key === "fbclid" || key === "gclid") {
      url.searchParams.delete(key);
    }
  }
  return url;
}
