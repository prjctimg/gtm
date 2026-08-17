## Spec: Download Retry & Resilience

### Problem
When a YouTube download failed, the user received an error immediately with
no retry attempt. Network hiccups or transient failures caused unnecessary
breakage.

### Changes

#### Download Retry (`daemon.rs`)
- `download_audio_to_cache` now retries up to 3 times
- 2-second backoff between attempts (exponential: 2s, 4s)
- Only surfaces error to user after the 3rd failure
- Internally delegates to `try_download_audio_to_cache`

### Verification
- Simulate a transient failure (e.g., temporary network issue)
- Verify retry attempts occur before final failure
- On success after retry, no error is shown to the user
