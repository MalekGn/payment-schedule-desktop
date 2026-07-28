// Unit tests for the tel:/sms: URI builder behind the call and message buttons
// on the Impayés page and the dashboard's overdue panel.
//
// `contactUri` is the only guard between the client form's free-text phone field
// and the OS URI handler, so these tests pin down both halves of its job:
// rejecting anything that isn't a dialable number, and reducing what it accepts
// to digits (plus an optional leading "+") before it reaches the URI. The
// surrounding `useContactActions` composable needs vue-i18n and Pinia, so the
// toast/failure wiring is covered by the E2E suite instead.

import { describe, it, expect } from "vitest";
import { contactUri } from "@/composables/useContactActions";

describe("contactUri — numbers people actually enter", () => {
  it("uses the tel: scheme for calls and sms: for messages", () => {
    expect(contactUri("call", "21698123456")).toBe("tel:21698123456");
    expect(contactUri("message", "21698123456")).toBe("sms:21698123456");
  });

  it("strips the separators people type, since they are presentational", () => {
    expect(contactUri("call", "98 123 456")).toBe("tel:98123456");
    expect(contactUri("call", "(216) 98-123.456")).toBe("tel:21698123456");
    expect(contactUri("call", " 98123456 ")).toBe("tel:98123456");
  });

  it("keeps a leading + so international numbers still dial", () => {
    expect(contactUri("call", "+216 98 123 456")).toBe("tel:+21698123456");
  });

  it("refuses a + that is not the international prefix, rather than fixing it", () => {
    expect(contactUri("call", "+216+98123456")).toBeNull();
  });
});

describe("contactUri — numbers that must not reach the OS handler", () => {
  it("rejects empty and whitespace-only input", () => {
    expect(contactUri("call", "")).toBeNull();
    expect(contactUri("call", "   ")).toBeNull();
  });

  it("rejects free text, so a note in the phone field cannot be dialed", () => {
    expect(contactUri("call", "appeler le bureau")).toBeNull();
    expect(contactUri("call", "N/A")).toBeNull();
  });

  it("rejects anything carrying another scheme or URI syntax", () => {
    // The phone field is free text; none of these may be spliced into a URI.
    expect(contactUri("call", "javascript:alert(1)")).toBeNull();
    expect(contactUri("call", "file:///etc/passwd")).toBeNull();
    expect(contactUri("call", "98123456?foo=bar")).toBeNull();
    expect(contactUri("call", "98123456#frag")).toBeNull();
    expect(contactUri("call", "98123456,999")).toBeNull();
  });

  it("rejects numbers too short to be a real phone number", () => {
    expect(contactUri("call", "12")).toBeNull();
    expect(contactUri("call", "+")).toBeNull();
  });

  it("rejects an implausibly long number rather than passing it through", () => {
    expect(contactUri("call", "9".repeat(40))).toBeNull();
  });

  it("rejects a separator-only string that would reduce to nothing", () => {
    expect(contactUri("call", "()-.")).toBeNull();
  });
});
