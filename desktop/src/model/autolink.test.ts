// The same cases as crates/markdown's autolink tests, so the desktop app links what the others do.

import { expect, test } from "bun:test";
import { findAutolinks } from "./autolink";

/** Each link as [its text, its href]. */
const links = (text: string) => findAutolinks(text).map((link) => [text.slice(link.start, link.end), link.href]);

function one(text: string): string[] {
  const found = links(text);
  expect(found.length).toBe(1);
  return found[0]!;
}

test("www links open over http", () => {
  expect(one("www.commonmark.org")).toEqual(["www.commonmark.org", "http://www.commonmark.org"]);
  expect(one("Visit www.commonmark.org/help for more information.")[0]).toBe("www.commonmark.org/help");
  expect(one("Visit www.commonmark.org.")[0]).toBe("www.commonmark.org");
  expect(one("Visit www.commonmark.org/a.b.")[0]).toBe("www.commonmark.org/a.b");
  expect(one("(www.example.com)")[0]).toBe("www.example.com");
  expect(links("wwww.example.com")).toEqual([]);
  expect(links("xwww.example.com")).toEqual([]);
  expect(links("www.")).toEqual([]);
  expect(links("WWW.example.com")).toEqual([]);
});

test("parentheses close inside the link only when opened there", () => {
  expect(one("www.google.com/search?q=Markup+(business)")[0]).toBe("www.google.com/search?q=Markup+(business)");
  expect(one("www.google.com/search?q=Markup+(business)))")[0]).toBe("www.google.com/search?q=Markup+(business)");
  expect(one("(www.google.com/search?q=Markup+(business))")[0]).toBe("www.google.com/search?q=Markup+(business)");
  expect(one("(www.google.com/search?q=Markup+(business)")[0]).toBe("www.google.com/search?q=Markup+(business)");
  expect(one("www.google.com/search?q=(business))+ok")[0]).toBe("www.google.com/search?q=(business))+ok");
});

test("entities and angle brackets end a link", () => {
  expect(one("www.google.com/search?q=commonmark&hl=en")[0]).toBe("www.google.com/search?q=commonmark&hl=en");
  expect(one("www.google.com/search?q=commonmark&hl;")[0]).toBe("www.google.com/search?q=commonmark");
  expect(one("www.commonmark.org/he<lp")[0]).toBe("www.commonmark.org/he");
});

test("urls keep their scheme", () => {
  expect(one("http://commonmark.org")).toEqual(["http://commonmark.org", "http://commonmark.org"]);
  expect(one("(Visit https://encrypted.google.com/search?q=Markup+(business))")[0]).toBe("https://encrypted.google.com/search?q=Markup+(business)");
  expect(one("Run it at http://localhost:3000/app.")[0]).toBe("http://localhost:3000/app");
  expect(one("HTTPS://Example.com/")[0]).toBe("HTTPS://Example.com/");
  expect(one("'https://example.com/a'")[0]).toBe("https://example.com/a");
  expect(links("xhttps://example.com")).toEqual([]);
  expect(links("https://")).toEqual([]);
  expect(links("https://-x.com")).toEqual([]);
  expect(links("ftp://example.com")).toEqual([]);
});

test("no underscore in the last two labels", () => {
  expect(one("www._xxx.yyy.zzz")[0]).toBe("www._xxx.yyy.zzz");
  expect(links("www.xxx.yyy._zzz")).toEqual([]);
  expect(links("www.xxx._yyy.zzz")).toEqual([]);
  expect(one("https://a.b/c_d_")[0]).toBe("https://a.b/c_d");
});

test("emails open over mailto", () => {
  expect(one("foo@bar.baz")).toEqual(["foo@bar.baz", "mailto:foo@bar.baz"]);
  expect(one("hello@mail+xyz.example isn't valid, but hello+xyz@mail.example is.")[0]).toBe("hello+xyz@mail.example");
  expect(one("a.b-c_d@a.b")[0]).toBe("a.b-c_d@a.b");
  expect(one("a.b-c_d@a.b.")[0]).toBe("a.b-c_d@a.b");
  expect(links("a.b-c_d@a.b-")).toEqual([]);
  expect(links("a.b-c_d@a.b_")).toEqual([]);
  expect(links("@bar.baz")).toEqual([]);
  expect(links("foo@bar")).toEqual([]);
  expect(links("path/to@bar.baz")).toEqual([]);
  expect(one("a@b@c.com")[0]).toBe("b@c.com");
});

test("several links in a run", () => {
  expect(links("see https://a.com, www.b.org and c@d.io.")).toEqual([
    ["https://a.com", "https://a.com"],
    ["www.b.org", "http://www.b.org"],
    ["c@d.io", "mailto:c@d.io"],
  ]);
});

test("CJK text runs into links", () => {
  expect(one("请访问https://example.com/docs，里面有说明。")[0]).toBe("https://example.com/docs");
  expect(one("见www.example.com。")[0]).toBe("www.example.com");
  expect(one("（https://zh.wikipedia.org/wiki/北京）")[0]).toBe("https://zh.wikipedia.org/wiki/北京");
  expect(one("“https://example.com”")[0]).toBe("https://example.com");
  expect(one("联系me@example.com。")[0]).toBe("me@example.com");
});
