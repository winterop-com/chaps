"""Predict with the example model: each future period of a location gets its mean."""

import argparse
import pickle

import pandas as pd


def main() -> None:
    parser = argparse.ArgumentParser(description="Predict with the trained model")
    parser.add_argument("--model", required=True, help="the trained model file")
    parser.add_argument("--historic", required=True, help="historic data CSV")
    parser.add_argument("--future", required=True, help="future data CSV")
    parser.add_argument("--output", required=True, help="file to write the predictions CSV to")
    parser.add_argument("--config", required=True, help="YAML file with the options")
    args = parser.parse_args()

    with open(args.model, "rb") as f:
        model = pickle.load(f)

    future = pd.read_csv(args.future)
    future["sample_0"] = future["location"].map(model["means"]).fillna(0.0)
    future.to_csv(args.output, index=False)
    print(f"predicted {len(future)} rows")


if __name__ == "__main__":
    main()
