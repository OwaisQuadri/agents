import csv


def write_rows(path, rows):
    with open(path, "w", newline="") as handle:
        csv.writer(handle).writerows(rows)
