"""Framework adapters for operator-lite. Each module is a thin port of the
corresponding repo port (operator-lite/ports/...) onto the shared gate
plumbing in operator_lite._core. Frameworks are imported lazily inside each
adapter, so none need to be installed unless you use that adapter."""
