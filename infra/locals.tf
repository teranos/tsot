locals {
  rave_fqdn = "${var.rave_subdomain}.${var.root_domain}"
  seer_fqdn = "${var.seer_subdomain}.${var.root_domain}"
  game_fqdn = "${var.game_subdomain}.${var.root_domain}"
}
