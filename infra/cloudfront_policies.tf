# Look up AWS-managed CloudFront policies by name instead of
# hardcoding magic UUIDs in cloudfront.tf. The IDs are stable but the
# names are documented and grep-able; using the data source is the
# pattern AWS recommends.
#
# Reference: https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/using-managed-cache-policies.html

data "aws_cloudfront_cache_policy" "caching_optimized" {
  name = "Managed-CachingOptimized"
}

data "aws_cloudfront_cache_policy" "caching_disabled" {
  name = "Managed-CachingDisabled"
}

data "aws_cloudfront_origin_request_policy" "all_viewer" {
  name = "Managed-AllViewer"
}
