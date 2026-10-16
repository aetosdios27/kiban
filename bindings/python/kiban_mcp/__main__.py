"""Run with python -m kiban_mcp DATABASE_DIRECTORY."""

import argparse
from kiban import Kiban, KibanError
from . import create_server


def main():
    parser = argparse.ArgumentParser(description="Kiban UTF-8 agent memory over MCP stdio")
    parser.add_argument("directory", help="database directory (created if absent)")
    args = parser.parse_args()
    try:
        with Kiban(args.directory) as db:
            create_server(db).run(transport="stdio")
    except KibanError as error:
        parser.exit(1, f"kiban-mcp: {error}\n")


if __name__ == "__main__":
    main()
