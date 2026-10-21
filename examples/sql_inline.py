"""Read a parameterized query and summarize a synthetic in-memory table."""

import sqlite3

MINIMUM_AMOUNT_INT = 10


def load_amounts_list(connection_info: sqlite3.Connection) -> list[int]:
    """Read amounts meeting the configured minimum.

    Args:
        connection_info (sqlite3.Connection): Synthetic in-memory database.
    Returns:
        list[int]: Amounts selected in ascending order.
    Warnings:
        Database errors propagate to the caller.
    """
    query_str = """SELECT amount
FROM records
WHERE amount >= ?
ORDER BY amount"""
    rows_list = connection_info.execute(query_str, (MINIMUM_AMOUNT_INT,))
    return [row_tuple[0] for row_tuple in rows_list]


def main() -> None:
    """Create sample records and print their selected total.

    Args:
        None: Uses synthetic values defined in this example.
    Returns:
        None: Prints the total amount.
    Warnings:
        Creates only an in-memory SQLite database.
    """
    with sqlite3.connect(":memory:") as connection_info:
        connection_info.execute("CREATE TABLE records (amount INTEGER)")
        connection_info.executemany(
            "INSERT INTO records VALUES (?)", [(5,), (10,), (25,)])
        amounts_list = load_amounts_list(connection_info)
        print(sum(amounts_list))


if __name__ == "__main__":
    main()
