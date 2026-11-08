import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AssistantMessage } from "./AssistantMessage";

const render = (text: string) =>
  renderToStaticMarkup(<AssistantMessage text={text} />);

describe("AssistantMessage", () => {
  it("renders bold text and lists", () => {
    const html = render("- **Sport:** racing");
    expect(html).toContain("<li><strong>Sport:</strong> racing</li>");
  });

  it("renders tables", () => {
    const html = render("| A | B |\n|---|---|\n| 1 | 2 |");
    expect(html).toContain("<table>");
    expect(html).toContain("<td>2</td>");
  });

  it("does not load images", () => {
    expect(render("![x](https://example.com/a.png?data=secret)")).not.toContain(
      "<img",
    );
  });

  it("does not run raw HTML", () => {
    const html = render('<img src="x" onerror="alert(1)"><script>1</script>');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<script");
  });

  it("opens links in a new tab and drops unsafe ones", () => {
    expect(render("[site](https://example.com)")).toContain('target="_blank"');
    expect(render("[bad](javascript:alert(1))")).not.toContain("javascript:");
  });
});
