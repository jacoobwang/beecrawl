# Avoid implicit retries for Job creation

CLI Job creation requests will not be retried implicitly because a transport failure can leave the server-side submission ambiguous and a retry may create a duplicate Job. Status reads may use bounded retries, blocking waits will default to 300 seconds with a one-second poll interval, and a timeout will preserve and report the Job ID when one is known.
