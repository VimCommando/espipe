# Elasticsearch document pipe

This context names the sources espipe reads and the destinations to which it writes documents.

## Language

**Input**:
A source from which espipe reads documents. One invocation can have one or more inputs.
_Avoid_: Input form when referring to an output destination

**Output target**:
The destination selected by the final positional argument. It can be Elasticsearch, a file, or standard output.
_Avoid_: Input form, host when the destination includes an index

**Elastic CLI context reference**:
A symbolic leading-dot reference to one application in the active or a named Elastic CLI context, such as `.es` or `.production.es`.
_Avoid_: Elastic URL, known host

**Context output**:
An Elasticsearch output target that combines an Elastic CLI context reference with an index, such as `.production.es:/logs`.
_Avoid_: Context input, elasticrc output
