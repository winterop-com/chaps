# Train the example model: the mean of the last `window` periods of each location.
#
# Rscript scripts/train.R --data data.csv --model model.rds --config config.yml

suppressPackageStartupMessages(library(yaml))

args <- commandArgs(trailingOnly = TRUE)
arg <- function(name) {
  at <- which(args == paste0("--", name))
  if (length(at) == 0) stop(sprintf("the argument --%s is missing", name))
  args[at + 1]
}

config <- yaml.load_file(arg("config"))
if (is.null(config)) config <- list()
# chap-core puts the options under user_option_values; chapkit puts them at the top.
options <- if (is.null(config$user_option_values)) config else config$user_option_values
window <- max(as.integer(if (is.null(options$window)) 3 else options$window), 1)

data <- read.csv(arg("data"))
data <- data[!is.na(data$disease_cases), ]
data <- data[order(data$time_period), ]
means <- tapply(data$disease_cases, data$location, function(cases) mean(tail(cases, window)))

saveRDS(list(means = as.list(means)), arg("model"))
cat(sprintf("trained on %d rows, %d locations, window %d\n", nrow(data), length(means), window))
