/** Profile id from a display name: lowercase, anything outside [a-z0-9_-] becomes "-", runs collapse, ends trimmed. */
export function slug(name: string): string {
  return name.trim().toLowerCase().replace(/[^a-z0-9_-]/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
}
