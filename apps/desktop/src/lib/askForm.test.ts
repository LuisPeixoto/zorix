import assert from "node:assert/strict";
import { test } from "node:test";
import type { AskField } from "./api.ts";
import { emptyForm, formValues, hostOf } from "./askForm.ts";

const field = (name: string, kind: AskField["kind"], required = false): AskField => ({ name, kind, required });
const fields: AskField[] = [
  field("repo", { type: "text" }, true),
  field("count", { type: "integer" }),
  field("ratio", { type: "number" }),
  field("kind", { type: "choice", options: ["a", "b"] }),
  field("sure", { type: "boolean" }),
];

test("empty optional fields are left out and booleans are always sent", () => {
  const typed = { ...emptyForm(fields), repo: "zorix" };
  assert.deepEqual(formValues(fields, typed), { values: { repo: "zorix", sure: false }, problem: null });
});

test("numbers and choices are converted and checked", () => {
  const typed = { repo: "x", count: " 3 ", ratio: "0.5", kind: "b", sure: true };
  assert.deepEqual(formValues(fields, typed).values, { repo: "x", count: 3, ratio: 0.5, kind: "b", sure: true });
  for (const [name, value] of [["count", "1.5"], ["count", "many"], ["ratio", "Infinity"], ["kind", "c"]]) {
    assert.equal(formValues(fields, { ...typed, [name]: value }).problem?.name, name, `${name}=${value}`);
  }
});

test("a required field left empty is the problem", () => {
  assert.equal(formValues(fields, { ...emptyForm(fields), repo: "  " }).problem?.name, "repo");
});

test("the host of a link", () => {
  assert.equal(hostOf("https://billing.example.com:8443/connect?x=1"), "billing.example.com:8443");
});
