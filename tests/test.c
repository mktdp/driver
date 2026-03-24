#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#if defined(_WIN32)
#include <windows.h>
#else
#include <unistd.h>
#endif

#include "../include/fingerprint.h"

static void portable_sleep_seconds(unsigned int seconds) {
#if defined(_WIN32)
    Sleep((DWORD)(seconds * 1000U));
#else
    sleep(seconds);
#endif
}

static double match_threshold_from_env(void) {
    const double default_threshold = 0.06;
    const char *raw = getenv("FP_MATCH_THRESHOLD");
    if (raw == NULL || raw[0] == '\0') {
        return default_threshold;
    }

    char *end = NULL;
    double parsed = strtod(raw, &end);
    if (end == raw || *end != '\0' || parsed < 0.0 || parsed > 1.0) {
        return default_threshold;
    }
    return parsed;
}

static int check_rc(const char *step, int32_t rc) {
    if (rc == FP_OK) {
        return 0;
    }
    fprintf(stderr, "  x %s failed: %s (%d)\n", step, fp_strerror(rc), rc);
    return 1;
}

int main(void) {
    FpDevice *dev = NULL;
    uint8_t *tmpl_a = NULL;
    uint8_t *tmpl_b = NULL;
    uintptr_t len_a = 0;
    uintptr_t len_b = 0;
    double score = 0.0;
    double threshold = match_threshold_from_env();
    int exit_code = 1;

    printf("=== MKTDP Driver — C Smoke Test ===\n\n");
    printf("Match threshold: %.4f (set FP_MATCH_THRESHOLD to override)\n\n", threshold);

    printf("[1/6] Opening scanner...\n");
    dev = fp_open();
    if (!dev) {
        fprintf(stderr, "  x fp_open failed: no supported scanner found\n");
        goto cleanup;
    }
    printf("  ok scanner opened.\n\n");

    printf("[2/6] Place your finger on the scanner... (10 second timeout)\n");
    if (check_rc("first scan", fp_scan_and_extract(dev, 10000, &tmpl_a, &len_a)) != 0) {
        goto cleanup;
    }
    printf("  ok template A: %lu bytes\n\n", (unsigned long)len_a);

    printf("[3/6] Lift finger, then place the SAME finger again...\n");
    portable_sleep_seconds(2);
    if (check_rc("second scan", fp_scan_and_extract(dev, 10000, &tmpl_b, &len_b)) != 0) {
        goto cleanup;
    }
    printf("  ok template B: %lu bytes\n\n", (unsigned long)len_b);

    printf("[4/6] Comparing templates...\n");
    if (check_rc("verify", fp_verify(tmpl_a, len_a, tmpl_b, len_b, &score)) != 0) {
        goto cleanup;
    }
    printf("  ok similarity score: %.4f\n", score);
    if (score >= threshold) {
        printf("  -> MATCH (score >= %.4f)\n\n", threshold);
    } else {
        printf("  -> NO MATCH (score < %.4f)\n\n", threshold);
    }

    printf("[5/6] Closing scanner...\n");
    fp_close(dev);
    dev = NULL;
    printf("  ok done.\n\n");

    printf("[6/6] Summary:\n");
    printf("  Template A size: %lu bytes\n", (unsigned long)len_a);
    printf("  Template B size: %lu bytes\n", (unsigned long)len_b);
    printf("\n=== C smoke test complete ===\n");

    exit_code = 0;

cleanup:
    if (tmpl_a != NULL && len_a > 0) {
        fp_free(tmpl_a, len_a);
        tmpl_a = NULL;
    }
    if (tmpl_b != NULL && len_b > 0) {
        fp_free(tmpl_b, len_b);
        tmpl_b = NULL;
    }
    if (dev != NULL) {
        fp_close(dev);
        dev = NULL;
    }
    return exit_code;
}
