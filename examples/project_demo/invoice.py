"""Small mathematical invoice example for local rename review."""


def calculate_total(tax_rate_float: float) -> float:
    """Return an illustrative total with a fixed base amount."""
    amount = 120
    tax_amount_float = amount * tax_rate_float
    return amount + tax_amount_float
