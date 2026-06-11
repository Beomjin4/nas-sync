/**
 * Minimal glob matcher for sync exclusion patterns.
 *
 * Supported syntax (per line in settings):
 *   `*`   any chars except `/`
 *   `?`   one char except `/`
 *   `**`  any chars including `/`
 *
 * Patterns match against the vault-relative path (forward slashes).
 * A pattern ending in `/` is shorthand for `<pattern>**`.
 */
export class ExcludeMatcher {
  private regexes: RegExp[];

  constructor(patterns: string[]) {
    this.regexes = patterns
      .map((p) => p.trim())
      .filter((p) => p.length > 0 && !p.startsWith("#"))
      .map((p) => globToRegExp(p.endsWith("/") ? p + "**" : p));
  }

  isExcluded(path: string): boolean {
    return this.regexes.some((r) => r.test(path));
  }
}

function globToRegExp(glob: string): RegExp {
  let re = "";
  let i = 0;
  while (i < glob.length) {
    const c = glob[i];
    if (c === "*") {
      if (glob[i + 1] === "*") {
        re += ".*";
        i += 2;
        // collapse a following slash into the .*
        if (glob[i] === "/") i += 1;
      } else {
        re += "[^/]*";
        i += 1;
      }
    } else if (c === "?") {
      re += "[^/]";
      i += 1;
    } else {
      re += escapeRegExp(c);
      i += 1;
    }
  }
  return new RegExp("^" + re + "$");
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
