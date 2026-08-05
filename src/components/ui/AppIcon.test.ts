// Unit tests for the only `v-html` in the application.
//
// `AppIcon` renders inline SVG through `v-html`, with an `eslint-disable
// vue/no-v-html` and a comment asserting it is safe: "`body` is a lookup into
// the static ICONS map above (props.name only picks a key), never user input, so
// there is no XSS surface." That claim is the thing worth testing — if a future
// change ever made `name` reach the markup instead of merely selecting from it,
// the disable comment would still be sitting there looking reassuring.
//
// So these tests pin the *argument*, not the artwork. The icon paths themselves
// are deliberately not snapshotted: that would churn on every design tweak while
// proving nothing about safety.

import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import AppIcon from "@/components/ui/AppIcon.vue";

const render = (props: { name: string; size?: number | string }) => mount(AppIcon, { props });

describe("AppIcon — the v-html safety argument", () => {
  it("renders an icon body for a name it knows", () => {
    const html = render({ name: "check" }).html();
    // Something was substituted — the lookup works at all.
    expect(html).toContain("<svg");
    expect(html.length).toBeGreaterThan("<svg></svg>".length);
  });

  it("renders an empty svg for a name it does not know", () => {
    // The `?? ""` fallback: an unknown key yields nothing, not a broken glyph.
    const wrapper = render({ name: "definitely-not-an-icon" });
    expect(wrapper.find("svg").exists()).toBe(true);
    expect(wrapper.find("svg").element.innerHTML).toBe("");
  });

  it("never lets the name itself reach the markup", () => {
    // The whole claim, stated as a test: `name` selects a key, it is not
    // interpolated. If this ever fails, the eslint-disable is a lie.
    const wrapper = render({ name: '<img src=x onerror="alert(1)">' });
    expect(wrapper.find("img").exists()).toBe(false);
    expect(wrapper.html()).not.toContain("onerror");
    expect(wrapper.find("svg").element.innerHTML).toBe("");
  });

  it("does not execute a script smuggled through the name", () => {
    const wrapper = render({ name: "<script>window.__pwned = true</script>" });
    expect(wrapper.find("script").exists()).toBe(false);
    expect((window as unknown as Record<string, unknown>).__pwned).toBeUndefined();
  });
});

describe("AppIcon — sizing and accessibility", () => {
  it("treats a bare number as pixels", () => {
    const svg = render({ name: "check", size: 20 }).find("svg");
    expect(svg.attributes("width")).toBe("20px");
    expect(svg.attributes("height")).toBe("20px");
  });

  it("passes a string size through so callers can use em units", () => {
    const svg = render({ name: "check", size: "1.5em" }).find("svg");
    expect(svg.attributes("width")).toBe("1.5em");
  });

  it("is always hidden from screen readers", () => {
    // Every icon in this app sits beside its own visible label, so announcing
    // it would just double the label up.
    expect(render({ name: "check" }).find("svg").attributes("aria-hidden")).toBe("true");
    expect(render({ name: "nope" }).find("svg").attributes("aria-hidden")).toBe("true");
  });
});
