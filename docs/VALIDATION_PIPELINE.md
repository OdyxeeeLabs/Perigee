# Request validation pipeline

`core/src/validation_pipeline.rs` runs every JSON request body through five stages and
stops at the first one that fails:

1. **Schema**: required fields, types, lengths, numeric ranges, allowed values, unknown
   fields. All schema problems are reported together.
2. **Sanitization**: strips `<script>` blocks, tags and control characters from every
   string (XSS) and rejects payloads that look like SQL injection. Parameterized queries
   remain the real protection against SQLi; this stage is defence in depth.
3. **Business rules**: conditional and cross-field rules (`required_if`,
   `mutually_exclusive`, `less_than`, `when`).
4. **Async checks**: validators that need I/O, such as uniqueness lookups
   (`AsyncValidator`).
5. **Transformation**: normalisation of the accepted value (`lowercase`,
   `default_value`).

## Error format

Always `400`:

```json
{ "error": { "code": "validation_failed", "message": "Request validation failed",
  "stage": "schema",
  "fields": [ { "field": "name", "code": "required", "message": "is required" } ] } }
```

Codes: `required`, `invalid_type`, `unknown_field`, `too_short`, `too_long`,
`out_of_range`, `invalid_choice`, `suspicious_input`, `conflict`,
`must_be_less_than`, plus any code a custom rule or async validator returns. The error
body never repeats the submitted values.

Failures are logged as a warning with the stage and `field:code` pairs only.

## Example

```rust
let pipeline = Pipeline::new(Schema::new(vec![
        FieldRule::string("name").required().len(1, 64),
        FieldRule::string("kind").one_of(&["basic", "managed"]),
        FieldRule::string("manager"),
    ]))
    .rule(rules::required_if("manager", "kind", "managed"))
    .async_validator(UniqueVaultName::new(pool))
    .transform(transforms::default_value("kind", json!("basic")));

let body = pipeline.run_async(request_json).await?; // Err -> ValidationFailure::to_json()
```
