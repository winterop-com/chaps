"""Train the example model: the mean of the last `window` periods of each location."""

import argparse
import pickle

import pandas as pd
import yaml


def main() -> None:
    parser = argparse.ArgumentParser(description="Train the model")
    parser.add_argument("--data", required=True, help="training data CSV")
    parser.add_argument("--model", required=True, help="file to write the trained model to")
    parser.add_argument("--config", required=True, help="YAML file with the options")
    args = parser.parse_args()

    with open(args.config) as f:
        config = yaml.safe_load(f) or {}
    # chap-core puts the options under user_option_values; chapkit puts them at the top.
    options = config.get("user_option_values", config)
    window = max(int(options.get("window", 3)), 1)

    data = pd.read_csv(args.data).sort_values("time_period")
    data = data.dropna(subset=["disease_cases"])
    means = data.groupby("location")["disease_cases"].apply(lambda cases: cases.tail(window).mean())

    with open(args.model, "wb") as f:
        pickle.dump({"means": means.to_dict()}, f)
    print(f"trained on {len(data)} rows, {len(means)} locations, window {window}")


if __name__ == "__main__":
    main()
