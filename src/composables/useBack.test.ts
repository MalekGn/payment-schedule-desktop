// Unit tests for the back-navigation decision behind every "Retour" button
// (NotFoundView, ClientDetailView, PurchaseDetailView).
//
// `useBack` itself needs a live router, so the branch that actually matters is
// extracted into the pure `shouldGoBack` helper: given `history.state.back` and
// a path→route-name resolver, decide whether `router.back()` leads somewhere
// useful or whether the caller should push its fallback route instead. These
// tests pin down the three ways that decision can go wrong — a fresh load with
// no history, a stored entry that is itself an unknown URL, and a resolver that
// rejects the stored value.

import { describe, it, expect } from "vitest";
import { shouldGoBack } from "@/composables/useBack";

// Stand-in for `router.resolve(path).name` over the real route table: anything
// the router does not have a record for falls through to the catch-all.
const KNOWN: Record<string, string> = {
  "/": "dashboard",
  "/clients": "clients",
  "/clients/3": "client-detail",
  "/achats": "achats",
};
const resolveName = (path: string) => KNOWN[path] ?? "not-found";

describe("shouldGoBack — when there is no usable history", () => {
  it("refuses on a fresh document load, where vue-router leaves state.back null", () => {
    expect(shouldGoBack(null, resolveName)).toBe(false);
  });

  it("refuses when state.back is absent entirely", () => {
    expect(shouldGoBack(undefined, resolveName)).toBe(false);
  });

  it("refuses an empty path, which is not a route we could navigate to", () => {
    expect(shouldGoBack("", resolveName)).toBe(false);
  });

  it("refuses non-string history values rather than trusting them", () => {
    // history.state is untyped storage — a number or object can turn up there.
    expect(shouldGoBack(42, resolveName)).toBe(false);
    expect(shouldGoBack({ path: "/clients" }, resolveName)).toBe(false);
  });
});

describe("shouldGoBack — when history points at a real page", () => {
  it("goes back to a list route", () => {
    expect(shouldGoBack("/clients", resolveName)).toBe(true);
  });

  it("goes back to a detail route", () => {
    expect(shouldGoBack("/clients/3", resolveName)).toBe(true);
  });

  it("goes back to the dashboard", () => {
    expect(shouldGoBack("/", resolveName)).toBe(true);
  });
});

describe("shouldGoBack — when history points at another unknown URL", () => {
  it("refuses, so Back never swaps one not-found screen for another", () => {
    expect(shouldGoBack("/nope", resolveName)).toBe(false);
  });

  it("refuses a deep unknown path, not just a single bad segment", () => {
    expect(shouldGoBack("/achats/12/nope", resolveName)).toBe(false);
  });

  it("treats a resolver that throws as no history rather than propagating", () => {
    const throwing = () => {
      throw new Error("malformed location");
    };
    expect(shouldGoBack("://not-a-path", throwing)).toBe(false);
  });
});
