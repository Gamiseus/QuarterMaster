export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

/** Within `slack` px of the bottom (or not scrollable at all): new lines keep the view at the bottom. */
export function isAtBottom(m: ScrollMetrics, slack = 16): boolean {
  return m.scrollHeight - m.scrollTop - m.clientHeight <= slack;
}
