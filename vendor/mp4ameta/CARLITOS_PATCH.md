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

`src/atom/util.rs`, `src/atom/mod.rs`, `src/error.rs`: return `InvalidTimescale`
for a zero movie/chapter timescale instead of dividing by zero. Bound chapter
titles by their sample sizes, handle zero/one-byte UTF-8 titles without reading
into the next sample, and reject odd UTF-16 lengths.

`src/atom/chpl.rs`: read chapter-list entries through an atom-bounded reader and
seek past remaining padding. The count is at most 255; allocating and reading
the entire atom could otherwise consume memory proportional to unrelated
padding. Keep truncated titles from reading bytes in the following atom.

These reader changes are covered by `import::chapters::tests`, including direct
parser errors, short/BOM titles, independent damaged chapter formats and a
padded chapter list read through a reader that rejects large buffer requests.
