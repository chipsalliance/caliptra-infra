// Licensed under the Apache-2.0 license

package runner

import (
	"context"
	"fmt"
	"log"
	"sort"
	"time"

	compute "cloud.google.com/go/compute/apiv1"
	"cloud.google.com/go/compute/apiv1/computepb"
	"google.golang.org/api/iterator"
	"google.golang.org/protobuf/proto"
)

const maxVmDuration = 61 * time.Hour

func cleanupInstances(ctx context.Context) error {
	instanceSvc, err := compute.NewInstancesRESTClient(ctx)
	if err != nil {
		return err
	}
	log.Printf("Cleanup Instances\n")
	instances := instanceSvc.List(ctx, &computepb.ListInstancesRequest{
		Zone:    gcpZone,
		Project: gcpProject,
		Filter:  proto.String("labels.gce-github-runner:* OR name=github-runner-image-builder"),
	})
	for {
		instance, err := instances.Next()
		if err == iterator.Done {
			break
		}
		if err != nil {
			return err
		}
		creationTime, err := time.Parse(time.RFC3339, instance.GetCreationTimestamp())
		if err != nil {
			log.Printf("Error parsing vm creation time: %v", err)
			continue
		}
		_, is_runner := instance.Labels["gce-github-runner"]
		if !is_runner && instance.GetName() != "github-runner-image-builder" {
			log.Printf("filter returned an unexpected instance: %v", instance.GetName())
			continue
		}
		instanceTooOld := creationTime.Add(maxVmDuration).Before(time.Now())

		shouldDelete := instanceTooOld || (is_runner && instance.GetStatus() == "TERMINATED")
		if shouldDelete {
			// Try to get guest attributes for better telemetry before deleting
			state := "UNKNOWN"
			if is_runner {
				attr, err := instanceSvc.GetGuestAttributes(ctx, &computepb.GetGuestAttributesInstanceRequest{
					Project:   gcpProject,
					Zone:      gcpZone,
					Instance:  instance.GetName(),
					QueryPath: proto.String("caliptra-github-ci/"),
				})
				if err == nil {
					items := attr.GetQueryValue().GetItems()
					runnerState := findItem(items, "runner-state")
					runnerError := findItem(items, "runner-error")
					state = fmt.Sprintf("runner-state=%v, runner-error=%v", runnerState, runnerError)
				} else {
					state = fmt.Sprintf("attributes-unavailable: %v", err)
				}
			}

			log.Printf("Deleting instance %v (status=%v, tooOld=%v, %v)", instance.GetName(), instance.GetStatus(), instanceTooOld, state)
			instanceSvc.Delete(ctx, &computepb.DeleteInstanceRequest{
				Zone:     gcpZone,
				Project:  gcpProject,
				Instance: instance.GetName(),
			})
		}
	}
	return nil
}

func cleanupImages(ctx context.Context) error {
	imageSvc, err := compute.NewImagesRESTClient(ctx)
	if err != nil {
		return err
	}
	log.Printf("Cleanup Images\n")
	iter := imageSvc.List(ctx, &computepb.ListImagesRequest{
		Project: gcpProject,
		Filter:  proto.String("labels.gce-github-runner:*"),
	})
	count := 0
	images := []string{}
	for {
		log.Printf("Calling next")
		image, err := iter.Next()
		if err == iterator.Done {
			break
		}
		if err != nil {
			return err
		}
		images = append(images, image.GetName())
		count++
	}
	sort.Sort(sort.Reverse(sort.StringSlice(images)))
	for i, image := range images {
		// Only keep the most recent 3 images
		if i >= 3 {
			imageSvc.Delete(ctx, &computepb.DeleteImageRequest{
				Project: gcpProject,
				Image:   image,
			})
		}
	}
	return nil
}

func Cleanup(ctx context.Context) error {
	err := cleanupInstances(ctx)
	if err != nil {
		return err
	}

	return cleanupImages(ctx)
}
