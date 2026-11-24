"""Sample Python module to test the +.py explorer filter in eojupyter."""

from dataclasses import dataclass


@dataclass(frozen=True)
class NotebookStats:
    cells: int
    outputs: int


def summarize(cells: int, outputs: int) -> NotebookStats:
    """Return a summary object for a rendered notebook."""
    return NotebookStats(cells=cells, outputs=outputs)


if __name__ == "__main__":
    print(summarize(cells=5, outputs=4))
