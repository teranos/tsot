# Private S3 bucket holding the static game bundle (`roam/dist/`).
# CloudFront reads it through an Origin Access Control (OAC) — the
# bucket itself stays private.

resource "aws_s3_bucket" "static" {
  bucket        = var.static_bucket_name
  force_destroy = true
}

resource "aws_s3_bucket_public_access_block" "static" {
  bucket = aws_s3_bucket.static.id

  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# Modern replacement for Origin Access Identity. Signed S3 reads from
# CloudFront only; the bucket never accepts public anonymous traffic.
resource "aws_cloudfront_origin_access_control" "static" {
  name                              = "${var.static_bucket_name}-oac"
  description                       = "OAC for roam static bucket"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

# Bucket policy that allows reads only from the static CloudFront
# distribution. Other AWS principals (and the public internet) are
# refused at S3 itself.
resource "aws_s3_bucket_policy" "static_cf_read" {
  bucket = aws_s3_bucket.static.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Sid       = "AllowCloudFrontServicePrincipalRead"
        Effect    = "Allow"
        Principal = { Service = "cloudfront.amazonaws.com" }
        Action    = ["s3:GetObject"]
        Resource  = "${aws_s3_bucket.static.arn}/*"
        Condition = {
          StringEquals = {
            "AWS:SourceArn" = aws_cloudfront_distribution.static.arn
          }
        }
      }
    ]
  })
}
