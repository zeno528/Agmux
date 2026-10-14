import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ManagementPageTitle } from "./ManagementPageTitle";

describe("ManagementPageTitle", () => {
  it("renders the shared title and total count", () => {
    const markup = renderToStaticMarkup(createElement(ManagementPageTitle, {
      icon: createElement("span"),
      title: "Skill",
      count: 81,
    }));
    expect(markup).toContain("Skill");
    expect(markup).toContain("·");
    expect(markup).toContain(">81<");
  });
});
