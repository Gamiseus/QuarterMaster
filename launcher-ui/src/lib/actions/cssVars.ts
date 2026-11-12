/**
 * Sets CSS custom properties on an element, for the few sizes that are computed at run time (the Mods
 * splitter width). Everything else comes from tokens; never write literal style strings in markup.
 */
export function cssVars(node: HTMLElement, vars: Record<string, string>) {
  let current: Record<string, string> = {};
  const apply = (next: Record<string, string>) => {
    for (const name of Object.keys(current)) if (!(name in next)) node.style.removeProperty(name);
    for (const [name, value] of Object.entries(next)) node.style.setProperty(name, value);
    current = next;
  };
  apply(vars);
  return { update: apply };
}
