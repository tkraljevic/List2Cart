// @ts-expect-error bun:test is provided by the Bun test runner
import { describe, expect, test } from "bun:test";
import type { ViewHelpers } from "../contracts";
import { renderAbout } from "./index";

const helpers: ViewHelpers = {
  escapeHtml: (v) =>
    String(v ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;"),
  formatDate: (v) => v,
  sharedListNames: () => [],
};

describe("renderAbout", () => {
  test("renders metadata version, EUPL-1.2 license, trusted icon, and escapes HTML", () => {
    const html = renderAbout(
      {
        name: "List2Cart <script>",
        version: "0.0.2-alpha",
        license: "EUPL-1.2",
        iconUrl: "/assets/app-icon.png",
      },
      helpers,
    );

    expect(html).toContain("Verzija 0.0.2-alpha");
    expect(html).toContain("EUPL-1.2");
    expect(html).not.toContain("MIT");
    expect(html).toContain('src="/assets/app-icon.png"');
    expect(html).toContain("List2Cart &lt;script&gt;");
    expect(html).not.toContain("<script>");
    expect(html).toContain('data-goto="sync"');
  });
});
