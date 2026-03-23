//go:build hardwaretests

package fingerprint

import (
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

func TestHardwareEnrollVerify(t *testing.T) {
	requireHardwareTests(t)

	dev, err := Open()
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer dev.Close()

	t.Log("place finger for first capture...")
	tmplA, err := EnrollWithTimeout(dev, 10_000)
	if err != nil {
		t.Fatalf("enroll A: %v", err)
	}
	t.Logf("template A size: %d", len(tmplA))

	t.Log("lift finger, then place same finger again...")
	time.Sleep(2 * time.Second)
	tmplB, err := EnrollWithTimeout(dev, 10_000)
	if err != nil {
		t.Fatalf("enroll B: %v", err)
	}
	t.Logf("template B size: %d", len(tmplB))

	score, err := Verify(tmplA, tmplB)
	if err != nil {
		t.Fatalf("verify: %v", err)
	}
	t.Logf("similarity score: %.4f", score)
}

func TestHardwareVerifyFromGoroutine(t *testing.T) {
	requireHardwareTests(t)

	dev, err := Open()
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer dev.Close()

	t.Log("place finger for first capture...")
	tmplA, err := EnrollWithTimeout(dev, 10_000)
	if err != nil {
		t.Fatalf("enroll A: %v", err)
	}

	t.Log("lift finger, then place same finger again...")
	time.Sleep(2 * time.Second)
	tmplB, err := EnrollWithTimeout(dev, 10_000)
	if err != nil {
		t.Fatalf("enroll B: %v", err)
	}

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
