# Predict with the example model: each future period of a location gets its mean.
#
# Rscript scripts/predict.R --model model.rds --historic historic.csv \
#   --future future.csv --output predictions.csv --config config.yml

args <- commandArgs(trailingOnly = TRUE)
arg <- function(name) {
  at <- which(args == paste0("--", name))
  if (length(at) == 0) stop(sprintf("the argument --%s is missing", name))
  args[at + 1]
}

model <- readRDS(arg("model"))
future <- read.csv(arg("future"))
means <- unlist(model$means)
future$sample_0 <- unname(means[as.character(future$location)])
future$sample_0[is.na(future$sample_0)] <- 0

write.csv(future, arg("output"), row.names = FALSE)
cat(sprintf("predicted %d rows\n", nrow(future)))
