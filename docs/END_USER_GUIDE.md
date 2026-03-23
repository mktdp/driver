# End-User Integration Guide

This project is a native Rust library with a C ABI. Your app/backend should use
these four flows:

1. Enrollment from multiple scans (recommended: 6 scans of the same finger)
2. Verification of a live scan against a stored enrollment template
3. Identification (find best user from many stored templates)
4. Continuous scanning for attendance/check-in scenarios

## Core C API Flows

### 1) Enrollment (6 scans -> 1 enrollment package)

Use `fp_enroll_multi`:

```c
uint8_t *enrolled = NULL;
size_t enrolled_len = 0;

int32_t rc = fp_enroll_multi(
    dev,
    10000,  // timeout per capture
    6,      // scan count
    4,      // max attempts per scan
    &enrolled,
    &enrolled_len
);

if (rc != FP_OK) {
    // handle error
}

// Store enrolled/enrolled_len in your DB.
// Later free:
fp_free(enrolled, enrolled_len);
```

What this does internally:
- Captures 6 templates from the same finger
- Retries recoverable capture failures per slot
- Stores all captures inside one opaque enrollment package
- `fp_verify` compares a live scan against all stored views and uses the best score

What the enrollment package actually is:
- Yes, it is just a byte array (`uint8_t*` + length).
- It is not a single merged fingerprint image.
- It is one container that holds multiple ISO fingerprint templates (one per successful enrollment scan).
- Current container header uses magic `FPM1` with a version byte, then per-template length + bytes.
- Treat it as opaque application data: store and send as-is, do not parse or edit it in app code.

### 2) Verification

```c
uint8_t *live = NULL;
size_t live_len = 0;
double score = 0.0;

int32_t rc = fp_scan_and_extract(dev, 10000, &live, &live_len);
if (rc == FP_OK) {
    rc = fp_verify(stored, stored_len, live, live_len, &score);
}

if (rc == FP_OK && score >= 0.06) {
    // match
}

fp_free(live, live_len);
```

### 3) Identification (scan once, search a template table)

Use `fp_identify` when you have many stored templates and need the best match:

```c
uint8_t *probe = NULL;
size_t probe_len = 0;
size_t match_index = SIZE_MAX;
double best_score = 0.0;

int32_t rc = fp_scan_and_extract(dev, 10000, &probe, &probe_len);
if (rc == FP_OK) {
    rc = fp_identify(
        probe, probe_len,            // probe template
        candidate_templates,         // const uint8_t*[]
        candidate_template_lens,     // size_t[]
        candidate_count,             // number of rows
        0.06,                        // threshold
        &match_index,
        &best_score
    );
}

if (rc == FP_OK && match_index != SIZE_MAX) {
    // matching row found at match_index
}

fp_free(probe, probe_len);
```

Notes:
- The driver does matching only; your app maps `match_index` to your own entity ID.
- If no candidate meets threshold, `match_index` is `SIZE_MAX`.

### 4) Continuous scanning (attendance/check-in)

Use `fp_scan_continuous` with a callback:

```c
bool on_scan(const uint8_t *tmpl, size_t len, void *user_data) {
    // lookup user by template match, mark attendance, etc.
    // return true to keep scanning, false to stop
    return true;
}

int32_t rc = fp_scan_continuous(
    dev,
    10000,   // timeout per capture
    0,       // max_scans: 0 = unlimited
    on_scan,
    NULL
);
```

Notes:
- Callback template pointer is borrowed and valid only during callback execution.
- Copy template bytes inside callback if you need to retain them.
- Recoverable capture errors are automatically skipped and scanning continues.

## Database Storage

Store templates as opaque binary bytes. Do not parse or mutate template payloads.

Suggested fields:
- `user_id`
- `finger_template` (BLOB/bytea)
- `created_at`
- optional: `finger_index`, `notes`

## Optional Go Test Wrapper

`go/fingerprint` exists for integration testing and wrappers. Production logic
should treat the native C ABI as the source of truth.

## Environment Variables

- `FP_MATCH_THRESHOLD` (default `0.06`)
- `FP_FINGER_DEBOUNCE_MS` (default `180`)
- `FP_CAPTURE_SETTLE_MS` (default `0`)
- `FP_CAPTURE_HOLD_MS` (default `0`)

## Operational Tips

- Enrollment: ask users to place/lift the same finger naturally each time.
- Attendance mode: keep scanner fixed and avoid moving it during sessions.
- Tune threshold with your own population and risk tolerance.
