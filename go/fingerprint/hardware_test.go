//go:build hardwaretests

package fingerprint

import (
	"errors"
	"os"
	"testing"
	"time"
)

func requireHardwareTests(t *testing.T) {
	t.Helper()
	if os.Getenv("FP_HARDWARE_TESTS") != "1" {
		t.Skip("set FP_HARDWARE_TESTS=1 to run hardware tests")
	}
}

func enrollWithRetry(t *testing.T, dev *Device, label string, attempts int, timeoutMs uint32) []byte {
	t.Helper()

	if attempts < 1 {
		attempts = 1
	}

	var lastErr error
	for i := 1; i <= attempts; i++ {
		t.Logf("%s attempt %d/%d...", label, i, attempts)
		tmpl, err := EnrollWithTimeout(dev, timeoutMs)
		if err == nil {
			return tmpl
		}

		lastErr = err
		var fpErr FpError
		if errors.As(err, &fpErr) && (fpErr.Code == ErrTimeout || fpErr.Code == ErrNoFinger) {
			t.Logf("%s attempt %d timed out/no finger, retrying...", label, i)
			continue
		}

		t.Fatalf("%s: %v", label, err)
	}

	t.Fatalf("%s: exhausted retries: %v", label, lastErr)
	return nil
}

func TestHardwareEnrollVerify(t *testing.T) {
	requireHardwareTests(t)

	dev, err := Open()
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer dev.Close()

	t.Log("place finger for first capture...")
	tmplA := enrollWithRetry(t, dev, "enroll A", 3, 10_000)
	t.Logf("template A size: %d", len(tmplA))

	t.Log("lift finger, then place same finger again...")
	time.Sleep(2 * time.Second)
	tmplB := enrollWithRetry(t, dev, "enroll B", 3, 10_000)
	t.Logf("template B size: %d", len(tmplB))

	score, err := Verify(tmplA, tmplB)
	if err != nil {
		t.Fatalf("verify: %v", err)
	}
	t.Logf("similarity score: %.4f", score)
}

func TestHardwareEnrollConsensus(t *testing.T) {
	requireHardwareTests(t)

	dev, err := Open()
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer dev.Close()

	t.Log("enrollment: place the SAME finger repeatedly for 6 scans...")
	tmpl, err := EnrollFromScans(dev, 6, 10_000, 4)
	if err != nil {
		t.Fatalf("enroll consensus: %v", err)
	}
	t.Logf("consensus template size: %d", len(tmpl))
	if len(tmpl) == 0 {
		t.Fatal("consensus template must not be empty")
	}
}

func TestHardwareVerifyFromGoroutine(t *testing.T) {
	requireHardwareTests(t)

	dev, err := Open()
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer dev.Close()

	t.Log("place finger for first capture...")
	tmplA := enrollWithRetry(t, dev, "enroll A", 3, 10_000)

	t.Log("lift finger, then place same finger again...")
	time.Sleep(2 * time.Second)
	tmplB := enrollWithRetry(t, dev, "enroll B", 3, 10_000)

	type result struct {
		score float64
		err   error
	}
	ch := make(chan result, 1)

	go func() {
		score, verifyErr := Verify(tmplA, tmplB)
		ch <- result{score: score, err: verifyErr}
	}()

	select {
	case r := <-ch:
		if r.err != nil {
			t.Fatalf("verify in goroutine: %v", r.err)
		}
		t.Logf("goroutine similarity score: %.4f", r.score)
	case <-time.After(5 * time.Second):
		t.Fatal("verify goroutine timed out")
	}
}
