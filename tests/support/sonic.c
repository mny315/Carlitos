/* Exercise pitch detection, allocation failures and flushing in bundled Sonic. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static void *blocked_buffer;
static int rejected_allocations;

static void *checked_realloc(void *pointer, size_t length) {
  if (pointer == blocked_buffer) {
    rejected_allocations++;
    return NULL;
  }
  return realloc(pointer, length);
}

#define realloc checked_realloc
#include "../../vendor/sonic/sonic.c"
#undef realloc

static int allocation_failure(int output, int flushing, float speed) {
  short input[4096] = {0};
  sonicStream stream = sonicCreateStream(16000, 1);
  if (!stream) {
    return 1;
  }
  sonicSetSpeed(stream, speed);
  if (flushing && !sonicWriteShortToStream(stream, input, 128)) {
    sonicDestroyStream(stream);
    return 1;
  }
  short **buffer = output ? &stream->outputBuffer : &stream->inputBuffer;
  int *size = output ? &stream->outputBufferSize : &stream->inputBufferSize;
  short *original = *buffer;
  int original_size = *size;
  blocked_buffer = original;
  rejected_allocations = 0;
  int result = flushing ? sonicFlushStream(stream)
                        : sonicWriteShortToStream(stream, input, 4096);
  int failed = result != 0 || rejected_allocations == 0 ||
               *buffer != original || *size != original_size;
  if (failed) {
    fprintf(stderr, "%s buffer, %s, %.2fx: result=%d rejected=%d retained=%d\n",
            output ? "output" : "input", flushing ? "flush" : "write", speed,
            result, rejected_allocations, *buffer == original);
  }
  /* Also clean up the allocation when testing the old broken implementation. */
  if (!*buffer) {
    *buffer = original;
  }
  blocked_buffer = NULL;
  sonicDestroyStream(stream);
  return failed;
}

static int flush_preserves_short_signal(void) {
  short input[128], output[512];
  for (unsigned i = 0; i < sizeof(input) / sizeof(input[0]); i++) {
    input[i] = 8000;
  }
  sonicStream stream = sonicCreateStream(16000, 1);
  if (!stream) {
    return 1;
  }
  sonicSetSpeed(stream, 0.5f);
  int ok = sonicWriteShortToStream(stream, input, 128) && sonicFlushStream(stream);
  int frames = ok ? sonicReadShortFromStream(stream, output, 512) : 0;
  int audible = 0;
  for (int i = 0; i < frames; i++) {
    audible += output[i] > 4000;
  }
  /* Slowing this short signal must extend the signal itself, not replace
     its tail with the padding that flush adds internally. */
  int failed = frames != 256 || audible < 192;
  if (failed) {
    fprintf(stderr, "Short signal flush: frames=%d audible=%d\n", frames, audible);
  }
  sonicDestroyStream(stream);
  return failed;
}

static int high_rate_pitch_search(void) {
  short input[6000];
  uint32_t random = 1;
  for (unsigned i = 0; i < sizeof(input) / sizeof(input[0]); i++) {
    random = random * 1664525u + 1013904223u;
    input[i] = (short)((int)(random >> 16) - 32768);
  }
  /* The refinement pass searches full-rate samples. At 192 kHz its periods
     can approach 3000 frames, exceeding the cross-products' 32-bit range. */
  int expected = 0, expected_min = 0, expected_max = 0;
  double minimum = 65536.0, maximum = -1.0;
  for (int period = 2500; period <= 2700; period++) {
    double difference = 0.0;
    for (int i = 0; i < period; i++) {
      difference += abs((int)input[i] - input[i + period]);
    }
    difference /= period;
    if (difference < minimum) {
      minimum = difference;
      expected = period;
      expected_min = (int)difference;
    }
    if (difference > maximum) {
      maximum = difference;
      expected_max = (int)difference;
    }
  }
  int minimum_difference, maximum_difference;
  int period = findPitchPeriodInRange(input, 2500, 2700, &minimum_difference,
                                    &maximum_difference);
  int failed = period != expected || minimum_difference != expected_min ||
               maximum_difference != expected_max;
  if (failed) {
    fprintf(stderr, "High-rate pitch: period=%d/%d min=%d/%d max=%d/%d\n",
            period, expected, minimum_difference, expected_min,
            maximum_difference, expected_max);
  }
  return failed;
}

int main(void) {
  int failed = flush_preserves_short_signal();
  failed |= high_rate_pitch_search();
  float speeds[] = {0.5f, 1.0f, 1.5f, 3.0f};
  for (unsigned i = 0; i < sizeof(speeds) / sizeof(speeds[0]); i++) {
    failed |= allocation_failure(0, 0, speeds[i]);
    failed |= allocation_failure(1, 0, speeds[i]);
    failed |= allocation_failure(0, 1, speeds[i]);
  }
  if (!failed) {
    puts("Sonic pitch, allocation failure and flush checks passed");
  }
  return failed;
}
