# Carlitos patch to mp4ameta 0.13.0

Source: the published `mp4ameta` 0.13.0 crate, under MIT OR Apache-2.0.
Upstream: https://github.com/saecki/mp4ameta

`src/atom/mod.rs`: convert the one-based `stsc.first_chunk` boundary to a
zero-based slice boundary while reading chapter tracks. The original reader
assigns one extra chunk to the preceding sample count, silently omitting or
misreading chapters when samples-per-chunk changes. Reject zero and backwards
boundaries before slicing; require the table to start with chunk one.

`src/tag/userdata/mod.rs`: remove redundant type parentheses in four arguments
so the locally built dependency does not introduce `unused_parens` warnings.

Regression: `import::chapters::tests::chapter_tracks_can_change_the_number_of_samples_per_chunk`
constructs a QuickTime chapter track with three chapters in two chunks. FFprobe
reads all three chapters; the unpatched crate returns only the first two.
