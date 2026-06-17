# Both certificates live in us-east-1 because CloudFront ONLY accepts
# ACM certs from that region. This is an AWS-side hard requirement,
# not a preference.

resource "aws_acm_certificate" "game" {
  provider = aws.us_east_1

  domain_name       = local.game_fqdn
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_acm_certificate" "relay" {
  provider = aws.us_east_1

  domain_name       = local.relay_fqdn
  validation_method = "DNS"

  lifecycle {
    create_before_destroy = true
  }
}

# Block the rest of the graph until DNS validation actually completes.
# The validation records in route53.tf trigger this.

resource "aws_acm_certificate_validation" "game" {
  provider = aws.us_east_1

  certificate_arn         = aws_acm_certificate.game.arn
  validation_record_fqdns = [for r in aws_route53_record.game_cert_validation : r.fqdn]
}

resource "aws_acm_certificate_validation" "relay" {
  provider = aws.us_east_1

  certificate_arn         = aws_acm_certificate.relay.arn
  validation_record_fqdns = [for r in aws_route53_record.relay_cert_validation : r.fqdn]
}
