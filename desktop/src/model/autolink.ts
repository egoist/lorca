// Bare links in message text, as `crates/markdown/src/autolink.rs` finds them for the macOS and
// phone apps: GitHub's autolinks (`http://` and `https://` URLs, `www.` hosts, email addresses),
// with a `www.` link allowed after any non-ASCII character and a link ending at CJK or general
// punctuation as it ends at a space. The rules and their cases are the crate's.

/** A bare link in a run of text: the range it covers and where it goes. */
export interface Autolink {
  start: number;
  end: number;
  href: string;
}

const alphanumeric = /[\p{Alphabetic}\p{N}]/u;
const whiteSpace = /\p{White_Space}/u;
const endsLink = /[\p{White_Space}< -⁯　-〿！-／：-＠［-｀｛-･]/gu;
const scheme = /https?:\/\//iy;
const asciiAlpha = /[A-Za-z]/;
const asciiAlphanumeric = /[A-Za-z0-9]/;
const nameChar = /[A-Za-z0-9.+\-_]/;

/** The bare links in `text`, in order. */
export function findAutolinks(text: string): Autolink[] {
  const links: Autolink[] = [];
  // The end of the last link: an address never reaches back into it.
  let floor = 0;
  for (let i = 0; i < text.length; ) {
    const c = text[i];
    const found = c === "h" || c === "H" ? url(text, i) : c === "w" ? www(text, i) : c === "@" ? email(text, i, floor) : null;
    if (found) {
      links.push(found);
      i = floor = found.end;
    } else {
      i++;
    }
  }
  return links;
}

/** `http://` or `https://`, in any case, then a host. GitHub reads back over letters to find the
 * scheme, so `xhttp://` is not a link. */
function url(text: string, start: number): Autolink | null {
  scheme.lastIndex = start;
  const head = scheme.exec(text);
  if (!head) return null;
  if (start > 0 && asciiAlpha.test(text[start - 1]!)) return null;
  const from = start + head[0].length;
  const first = text.codePointAt(from);
  if (first === undefined || !alphanumeric.test(String.fromCodePoint(first)) || !host(text, from)) return null;
  const end = start + trim(text.slice(start, extend(text, start)));
  return { start, end, href: text.slice(start, end) };
}

/** `www.` and a host, at the start of the text or after a space, one of `*_~(`, or a non-ASCII
 * character. It opens over `http://`. */
function www(text: string, start: number): Autolink | null {
  if (!text.startsWith("www.", start)) return null;
  if (start > 0) {
    const c = text[start - 1]!;
    if (!(whiteSpace.test(c) || "*_~(".includes(c) || c.charCodeAt(0) > 0x7f)) return null;
  }
  if (!host(text, start)) return null;
  const end = start + trim(text.slice(start, extend(text, start)));
  if (end <= start + 4) return null;
  return { start, end, href: `http://${text.slice(start, end)}` };
}

/** An address around the `@` at `at`: letters, digits, and `.+-_` before it, and after it labels
 * of letters, digits, `-`, and `_` between dots, at least two, ending in a letter. It opens over
 * `mailto:`. One that follows a `/` is part of a path, not an address. */
function email(text: string, at: number, floor: number): Autolink | null {
  let start = at;
  while (start > floor && nameChar.test(text[start - 1]!)) start--;
  if (start === at || text[start - 1] === "/") return null;
  let end = at + 1;
  let dots = 0;
  for (; end < text.length; end++) {
    const c = text[end]!;
    if (c === "-" || c === "_" || asciiAlphanumeric.test(c)) continue;
    if (c === "." && asciiAlphanumeric.test(text[end + 1] ?? "")) {
      dots++;
      continue;
    }
    if (c === "@") return null;
    break;
  }
  if (dots === 0 || !asciiAlpha.test(text[end - 1]!)) return null;
  return { start, end, href: `mailto:${text.slice(start, end)}` };
}

/** Whether the host name at `from`, letters, digits, hyphens, and underscores between dots, has no
 * underscore in its last two labels. */
function host(text: string, from: number): boolean {
  // Whether the label being read, and the one before it, has an underscore.
  let last = false;
  let previous = false;
  for (let i = from; i < text.length; ) {
    const c = String.fromCodePoint(text.codePointAt(i)!);
    i += c.length;
    if (c === ".") {
      previous = last;
      last = false;
    } else if (c === "_") {
      last = true;
    } else if (c !== "-" && !alphanumeric.test(c)) {
      break;
    }
  }
  return !last && !previous;
}

/** Where a link that starts at `start` runs to: the next space, `<`, or CJK or general punctuation. */
function extend(text: string, start: number): number {
  endsLink.lastIndex = start;
  return endsLink.exec(text)?.index ?? text.length;
}

/** The length of `link` without its trailing punctuation: `?!.,:*_~'"`, a `)` with no `(` to
 * close, a `;`, or an entity such as `&hl;` whole. */
function trim(link: string): number {
  const opening = link.split("(").length - 1;
  let closing = link.split(")").length - 1;
  let end = link.length;
  while (end > 0) {
    const c = link[end - 1]!;
    if (c === ")" && closing > opening) {
      closing--;
      end--;
    } else if ("?!.,:*_~'\"".includes(c)) {
      end--;
    } else if (c === ";") {
      let name = 0;
      while (end - name - 2 >= 0 && asciiAlpha.test(link[end - name - 2]!)) name++;
      if (name > 0 && end >= name + 2 && link[end - name - 2] === "&") end -= name + 2;
      else end--;
    } else {
      break;
    }
  }
  return end;
}
