// Validate an assembled content.json against the landing's content-contract JSON schema.
// Fail-closed AT THE SOURCE (§4): a contract break stops in this repo's CI, never at the consumer.
// The schema is draft 2020-12; `strict:false` because we validate against a foreign schema whose
// constructs (if/then over a discriminated union, additionalProperties:true) must not trip ajv's
// own authoring strictness.

import Ajv2020 from "ajv/dist/2020.js";

/** @returns {{ok:boolean, errors:object[]}} */
export function validateContent(manifest, schema) {
  const ajv = new Ajv2020({ allErrors: true, strict: false });
  const validate = ajv.compile(schema);
  const ok = validate(manifest) === true;
  return { ok, errors: validate.errors ?? [] };
}

/** Human-legible one-line-per-error rendering for CI logs. */
export function formatErrors(errors) {
  return (errors ?? [])
    .map((e) => `  ${e.instancePath || "(root)"} ${e.message}${e.params ? " " + JSON.stringify(e.params) : ""}`)
    .join("\n");
}
