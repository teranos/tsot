# psychostasia — private bucket holding the piece's media (the fire clip,
# the brass scale model and its textures). Media never goes into git and
# is never served publicly: `make assets` in psychostasia/ pulls it and
# checks every file against psychostasia/assets.sha256.
# No force_destroy: unlike game's dist/, this media can't be regenerated.

variable "psychostasia_bucket_name" {
  description = "Private S3 bucket for psychostasia's media. Global namespace; suffixed with the account id to stay unique."
  type        = string
  default     = "psychostasia-assets-548351057127"
}

resource "aws_s3_bucket" "psychostasia_assets" {
  bucket = var.psychostasia_bucket_name
}

resource "aws_s3_bucket_public_access_block" "psychostasia_assets" {
  bucket = aws_s3_bucket.psychostasia_assets.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# An overwritten file stays recoverable.
resource "aws_s3_bucket_versioning" "psychostasia_assets" {
  bucket = aws_s3_bucket.psychostasia_assets.id

  versioning_configuration {
    status = "Enabled"
  }
}

output "psychostasia_assets_bucket" {
  description = "Private bucket `make assets` in psychostasia/ pulls media from."
  value       = aws_s3_bucket.psychostasia_assets.bucket
}
